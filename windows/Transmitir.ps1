# Sala de Tela: transmissor para Windows.
# Instalado pelo Instalar-Sala.bat em C:\ProgramData\SalaDeTela\Transmitir.ps1.
#
# Captura a tela principal (1080p60), codifica no encoder de hardware disponível
# e envia via WHIP para o servidor. O som do PC vai SEM o Discord
# (captura por processo do Windows, modo "excluir árvore de processos").
# Se o Discord não for encontrado, transmite sem som: nunca há eco.

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
[System.Windows.Forms.Application]::EnableVisualStyles()

# Preenchidos pelo build-instalador.sh
$Servidor = '__IP_SERVIDOR__'
$AudioPermitido = [bool]::Parse('__AUDIO_PERMITIDO__')

$PastaConfig = Join-Path $env:APPDATA 'SalaDeTela'
$ArqConfig = Join-Path $PastaConfig 'config.json'
$ArqLog = Join-Path $PastaConfig 'transmissao.log'
New-Item -ItemType Directory -Force -Path $PastaConfig | Out-Null

Add-Type -TypeDefinition @'
public static class SalaLog {
    public static void Ligar(System.Diagnostics.Process p, string arquivo) {
        var w = new System.IO.StreamWriter(arquivo, false, new System.Text.UTF8Encoding(false));
        w.AutoFlush = true;
        System.Diagnostics.DataReceivedEventHandler h = (s, e) => { if (e.Data != null) lock (w) w.WriteLine(e.Data); };
        p.OutputDataReceived += h;
        p.ErrorDataReceived += h;
        p.BeginOutputReadLine();
        p.BeginErrorReadLine();
    }
}
'@

# ------------------------------------------------------------------ config

function Ler-Config {
    $padrao = [pscustomobject]@{ nome = ''; encoder = $null; gstVersao = ''; comSom = $true }
    if (Test-Path $ArqConfig) {
        try {
            $c = Get-Content $ArqConfig -Raw -Encoding UTF8 | ConvertFrom-Json
            foreach ($p in $padrao.PSObject.Properties.Name) {
                if (-not ($c.PSObject.Properties.Name -contains $p)) { $c | Add-Member $p $padrao.$p }
            }
            return $c
        } catch { }
    }
    return $padrao
}

function Salvar-Config($c) {
    $c | ConvertTo-Json -Depth 5 | Set-Content -Path $ArqConfig -Encoding UTF8
}

function Nome-Seguro([string]$nome) {
    $n = $nome.ToLowerInvariant().Normalize([Text.NormalizationForm]::FormD)
    $n = ($n.ToCharArray() | Where-Object { [Globalization.CharUnicodeInfo]::GetUnicodeCategory($_) -ne 'NonSpacingMark' }) -join ''
    $n = ($n -replace '[^a-z0-9_-]+', '-').Trim('-')
    return $n
}

# ------------------------------------------------------------------ GStreamer

function Achar-Gst {
    $raizes = @(
        [Environment]::GetEnvironmentVariable('GSTREAMER_1_0_ROOT_MSVC_X86_64', 'Machine'),
        (Join-Path $env:ProgramFiles 'gstreamer\1.0\msvc_x86_64'),
        'C:\gstreamer\1.0\msvc_x86_64'
    ) | Where-Object { $_ }
    foreach ($r in $raizes) {
        $bin = Join-Path $r 'bin'
        if (Test-Path (Join-Path $bin 'gst-launch-1.0.exe')) { return $bin }
    }
    return $null
}

$GstBin = Achar-Gst
$GstLaunch = if ($GstBin) { Join-Path $GstBin 'gst-launch-1.0.exe' } else { $null }
$GstInspect = if ($GstBin) { Join-Path $GstBin 'gst-inspect-1.0.exe' } else { $null }

function Rodar([string]$exe, [string]$argumentos, [int]$limiteMs) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.Arguments = $argumentos
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $p = [System.Diagnostics.Process]::Start($psi)
    $out = $p.StandardOutput.ReadToEndAsync()
    $err = $p.StandardError.ReadToEndAsync()
    if (-not $p.WaitForExit($limiteMs)) {
        try { $p.Kill() } catch { }
        return [pscustomobject]@{ ok = $false; saida = 'tempo esgotado' }
    }
    return [pscustomobject]@{ ok = ($p.ExitCode -eq 0); saida = ($out.Result + $err.Result) }
}

function Tem-Elemento([string]$elemento) {
    return (Rodar $GstInspect "--exists $elemento" 120000).ok
}

# Encoders H.264 em ordem de preferência. "d3d" = aceita textura da GPU direto.
# Cada um tem uma lista de propriedades: a completa e uma mínima de reserva.
$Encoders = @(
    @{ nome = 'NVIDIA';                      el = 'nvd3d11h264enc'; d3d = $true;  w = 1920; h = 1080
       props = @('bitrate=9000 gop-size=60 bframes=0 rc-mode=cbr zero-reorder-delay=true', 'bitrate=9000') },
    @{ nome = 'AMD';                         el = 'amfh264enc';     d3d = $true;  w = 1920; h = 1080
       props = @('bitrate=9000 gop-size=60 rate-control=cbr usage=low-latency', 'bitrate=9000') },
    @{ nome = 'Intel';                       el = 'qsvh264enc';     d3d = $true;  w = 1920; h = 1080
       props = @('bitrate=9000 gop-size=60 b-frames=0 rate-control=cbr', 'bitrate=9000') },
    @{ nome = 'NVIDIA (CUDA)';               el = 'nvh264enc';      d3d = $false; w = 1920; h = 1080
       props = @('bitrate=9000 gop-size=60 bframes=0 rc-mode=cbr', 'bitrate=9000') },
    @{ nome = 'Windows (Media Foundation)';  el = 'mfh264enc';      d3d = $false; w = 1920; h = 1080
       props = @('bitrate=9000 gop-size=60 bframes=0 rc-mode=cbr low-latency=true', 'bitrate=9000') },
    @{ nome = 'Software (x264, 720p)';       el = 'x264enc';        d3d = $false; w = 1280; h = 720
       props = @('bitrate=4500 tune=zerolatency speed-preset=veryfast key-int-max=60 bframes=0') }
)

function Detectar-Encoder([scriptblock]$progresso) {
    foreach ($e in $Encoders) {
        if (-not (Tem-Elemento $e.el)) { continue }
        foreach ($props in $e.props) {
            & $progresso "Testando o codificador $($e.nome)…"
            $conv = if ($e.d3d) { '' } else { ' ! d3d11download ! videoconvert ! video/x-raw,format=NV12' }
            $teste = "-q d3d11testsrc num-buffers=30 ! video/x-raw(memory:D3D11Memory),width=$($e.w),height=$($e.h),framerate=60/1$conv ! $($e.el) $props ! h264parse ! fakesink"
            if ((Rodar $GstLaunch $teste 60000).ok) {
                return [pscustomobject]@{ nome = $e.nome; el = $e.el; props = $props; d3d = $e.d3d; w = $e.w; h = $e.h }
            }
        }
    }
    return $null
}

# ------------------------------------------------------------------ Discord

# Processos "raiz" do Discord (o Discord.exe cujo pai não é outro Discord.exe).
# Excluir a árvore desse processo tira todo o som do Discord da captura.
function Achar-Discord {
    $padrao = '^(Discord|DiscordPTB|DiscordCanary|DiscordDevelopment|Vesktop|Legcord)\.exe$'
    $procs = @(Get-CimInstance Win32_Process | Where-Object { $_.Name -match $padrao })
    if ($procs.Count -eq 0) { return @() }
    $ids = $procs | ForEach-Object { $_.ProcessId }
    return @($procs | Where-Object { $ids -notcontains $_.ParentProcessId } | ForEach-Object { [int]$_.ProcessId })
}

# ------------------------------------------------------------------ transmissão

$Script:Proc = $null
$Script:PidDiscord = 0
$Script:ComSom = $false
$Script:Encoder = $null

function Montar-Pipeline([string]$nome, [bool]$comSom, [int]$pidDiscord) {
    $e = $Script:Encoder
    $endpoint = "http://${Servidor}:8889/$nome/whip"
    $sink = "whipclientsink name=ws congestion-control=disabled signaller::whip-endpoint=$endpoint"
    $video = "d3d11screencapturesrc show-cursor=true ! video/x-raw(memory:D3D11Memory),framerate=60/1" +
             " ! d3d11convert ! video/x-raw(memory:D3D11Memory),width=$($e.w),height=$($e.h),pixel-aspect-ratio=1/1"
    if (-not $e.d3d) { $video += ' ! d3d11download ! videoconvert ! video/x-raw,format=NV12' }
    $video += " ! queue max-size-buffers=2 leaky=downstream ! $($e.el) $($e.props) ! h264parse config-interval=-1 ! ws."
    $audio = ''
    if ($comSom) {
        $audio = " wasapi2src loopback=true low-latency=true loopback-mode=exclude-process-tree loopback-target-pid=$pidDiscord" +
                 ' ! queue ! audioconvert ! audioresample ! audio/x-raw,rate=48000,channels=2 ! opusenc bitrate=128000 ! ws.'
    }
    return "-e $sink $video$audio"
}

function Parar-Transmissao {
    if ($Script:Proc -and -not $Script:Proc.HasExited) {
        try { $Script:Proc.Kill() } catch { }
        $Script:Proc.WaitForExit(3000) | Out-Null
    }
    $Script:Proc = $null
}

function Iniciar-Transmissao([string]$nome, [bool]$querSom) {
    $raizes = Achar-Discord
    $Script:ComSom = $false
    $Script:PidDiscord = 0
    $avisoSom = ''
    if (-not $AudioPermitido) {
        $avisoSom = 'Sem som (desativado nesta instalação).'
    } elseif (-not $querSom) {
        $avisoSom = 'Sem som (você desmarcou a opção).'
    } elseif ($raizes.Count -eq 1) {
        $Script:ComSom = $true
        $Script:PidDiscord = $raizes[0]
        $avisoSom = 'Com o som do PC, sem o Discord.'
    } elseif ($raizes.Count -eq 0) {
        $avisoSom = 'Sem som: o app do Discord não está aberto. (Abra o Discord e o som liga sozinho.)'
    } else {
        $avisoSom = 'Sem som: há mais de um Discord aberto. Feche os extras para ter som.'
    }

    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $GstLaunch
    $psi.Arguments = Montar-Pipeline $nome $Script:ComSom $Script:PidDiscord
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.EnvironmentVariables['GST_DEBUG'] = '2'
    $psi.EnvironmentVariables['GST_DEBUG_NO_COLOR'] = '1'
    $Script:Proc = [System.Diagnostics.Process]::Start($psi)
    [SalaLog]::Ligar($Script:Proc, $ArqLog)
    "[$(Get-Date -Format s)] gst-launch-1.0 $($psi.Arguments)" | Out-File -FilePath "$ArqLog.cmd" -Encoding UTF8
    return $avisoSom
}

# ------------------------------------------------------------------ diagnóstico

function Gerar-Diagnostico {
    $l = New-Object System.Collections.Generic.List[string]
    $l.Add("Sala de Tela: diagnóstico $(Get-Date -Format s)")
    $os = Get-CimInstance Win32_OperatingSystem
    $l.Add("Windows: $($os.Caption) build $($os.BuildNumber) (captura por processo exige build >= 20348)")
    $l.Add("Servidor: $Servidor")
    try {
        $r = Invoke-WebRequest -UseBasicParsing -TimeoutSec 5 "http://${Servidor}:8080/api/ao-vivo"
        $l.Add("Servidor alcançável: sim ($($r.Content))")
    } catch { $l.Add("Servidor alcançável: NÃO ($($_.Exception.Message))") }
    $ts = Join-Path $env:ProgramFiles 'Tailscale\tailscale.exe'
    if (Test-Path $ts) { $l.Add("Tailscale IP: $(& $ts ip -4 2>&1)") } else { $l.Add('Tailscale: NÃO instalado') }
    $l.Add("GStreamer: $GstBin")
    if ($GstBin) {
        $l.Add((Rodar $GstLaunch '--version' 60000).saida.Trim())
        foreach ($el in 'd3d11screencapturesrc', 'd3d11testsrc', 'd3d11convert', 'whipclientsink', 'wasapi2src', 'opusenc', 'h264parse',
                        'nvd3d11h264enc', 'amfh264enc', 'qsvh264enc', 'nvh264enc', 'mfh264enc', 'x264enc') {
            $l.Add(("  {0,-24} {1}" -f $el, $(if (Tem-Elemento $el) { 'ok' } else { 'ausente' })))
        }
        $w = (Rodar $GstInspect 'wasapi2src' 60000).saida
        $l.Add("  wasapi2src loopback-mode (exclusão do Discord): $(if ($w -match 'loopback-mode') { 'disponível' } else { 'INDISPONÍVEL' })")
    }
    $cfg = Ler-Config
    $l.Add("Encoder escolhido: $(if ($cfg.encoder) { "$($cfg.encoder.nome) [$($cfg.encoder.el) $($cfg.encoder.props)]" } else { 'nenhum ainda' })")
    $l.Add("Discord (processos raiz): $((Achar-Discord) -join ', ')")
    if (Test-Path "$ArqLog.cmd") { $l.Add(''); $l.Add('Último comando:'); $l.Add((Get-Content "$ArqLog.cmd" -Raw)) }
    if (Test-Path $ArqLog) { $l.Add('Fim do log da última transmissão:'); $l.AddRange([string[]](Get-Content $ArqLog -Tail 40)) }
    $arq = Join-Path ([Environment]::GetFolderPath('Desktop')) 'SalaDeTela-diagnostico.txt'
    $l | Set-Content -Path $arq -Encoding UTF8
    Start-Process notepad.exe $arq
}

# ------------------------------------------------------------------ janela

$cfg = Ler-Config
if (-not $cfg.nome) { $cfg.nome = Nome-Seguro $env:USERNAME }

$form = New-Object System.Windows.Forms.Form
$form.Text = 'Sala de Tela: Transmitir minha tela'
$form.Size = New-Object System.Drawing.Size(460, 330)
$form.FormBorderStyle = 'FixedDialog'
$form.MaximizeBox = $false
$form.StartPosition = 'CenterScreen'
$form.Font = New-Object System.Drawing.Font('Segoe UI', 10)

$lblNome = New-Object System.Windows.Forms.Label
$lblNome.Text = 'Seu nome na Sala:'
$lblNome.Location = '20,20'; $lblNome.AutoSize = $true
$form.Controls.Add($lblNome)

$txtNome = New-Object System.Windows.Forms.TextBox
$txtNome.Text = $cfg.nome
$txtNome.Location = '160,17'; $txtNome.Width = 260
$form.Controls.Add($txtNome)

$chkSom = New-Object System.Windows.Forms.CheckBox
$chkSom.Text = 'Transmitir o som do PC (o Discord fica de fora)'
$chkSom.Location = '20,55'; $chkSom.AutoSize = $true
$chkSom.Checked = [bool]$cfg.comSom -and $AudioPermitido
$chkSom.Enabled = $AudioPermitido
$form.Controls.Add($chkSom)

$btn = New-Object System.Windows.Forms.Button
$btn.Text = '▶  Transmitir'
$btn.Location = '20,90'; $btn.Size = '400,50'
$btn.Font = New-Object System.Drawing.Font('Segoe UI', 13, [System.Drawing.FontStyle]::Bold)
$form.Controls.Add($btn)

$lblStatus = New-Object System.Windows.Forms.Label
$lblStatus.Location = '20,150'; $lblStatus.Size = '400,80'
$lblStatus.Text = 'Pronto para transmitir a sua tela principal.'
$form.Controls.Add($lblStatus)

$lnkSala = New-Object System.Windows.Forms.LinkLabel
$lnkSala.Text = 'Abrir a Sala (assistir)'
$lnkSala.Location = '20,245'; $lnkSala.AutoSize = $true
$lnkSala.add_LinkClicked({ Start-Process "http://${Servidor}:8080/" })
$form.Controls.Add($lnkSala)

$btnDiag = New-Object System.Windows.Forms.Button
$btnDiag.Text = 'Diagnóstico'
$btnDiag.Location = '300,240'; $btnDiag.Size = '120,30'
$form.Controls.Add($btnDiag)

function Status([string]$texto) {
    $lblStatus.Text = $texto
    [System.Windows.Forms.Application]::DoEvents()
}

$Script:Transmitindo = $false
$Script:Falhas = 0
$Script:AvisoSom = ''

function Ligar {
    $nome = Nome-Seguro $txtNome.Text
    if (-not $nome) { Status 'Escreva um nome (letras e números).'; return }
    $txtNome.Text = $nome
    if (-not $GstLaunch) { Status 'GStreamer não encontrado. Rode o instalador da Sala de novo.'; return }

    $btn.Enabled = $false
    $versao = (Rodar $GstLaunch '--version' 120000).saida.Trim()
    if (-not $cfg.encoder -or $cfg.gstVersao -ne $versao) {
        foreach ($el in 'd3d11screencapturesrc', 'whipclientsink') {
            if (-not (Tem-Elemento $el)) { Status "Falta o componente '$el' do GStreamer. Clique em Diagnóstico e mande o arquivo para o dono da Sala."; $btn.Enabled = $true; return }
        }
        $cfg.encoder = Detectar-Encoder { param($m) Status $m }
        $cfg.gstVersao = $versao
        if (-not $cfg.encoder) { Status 'Nenhum codificador de vídeo funcionou. Clique em Diagnóstico.'; $btn.Enabled = $true; return }
    }
    $cfg.nome = $nome
    $cfg.comSom = $chkSom.Checked
    Salvar-Config $cfg
    $Script:Encoder = $cfg.encoder

    $Script:AvisoSom = Iniciar-Transmissao $nome $chkSom.Checked
    $Script:Transmitindo = $true
    $txtNome.Enabled = $false; $chkSom.Enabled = $false
    $btn.Text = '■  Parar'
    $btn.Enabled = $true
    Status "AO VIVO como '$nome' ($($Script:Encoder.w)x$($Script:Encoder.h) 60fps, $($Script:Encoder.nome)).`n$($Script:AvisoSom)"
}

function Desligar([string]$motivo) {
    $Script:Transmitindo = $false
    Parar-Transmissao
    $txtNome.Enabled = $true; $chkSom.Enabled = $AudioPermitido
    $btn.Text = '▶  Transmitir'
    Status $motivo
}

$btn.add_Click({
    if ($Script:Transmitindo) { Desligar 'Transmissão encerrada.' } else { $Script:Falhas = 0; Ligar }
})
$btnDiag.add_Click({ Status 'Gerando diagnóstico…'; Gerar-Diagnostico; Status 'Diagnóstico salvo na Área de Trabalho.' })

# Vigia a cada 1 s: transmissão caiu? Discord fechou/abriu? (o PID excluído precisa ser o atual)
$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = 1000
$timer.add_Tick({
    if (-not $Script:Transmitindo) { return }
    if ($Script:Proc.HasExited) {
        $Script:Falhas++
        if ($Script:Falhas -gt 3) { Desligar 'A transmissão caiu várias vezes. O PC do servidor está ligado? Clique em Diagnóstico.'; return }
        Status "A transmissão caiu; tentando de novo ($($Script:Falhas)/3)…"
        $Script:AvisoSom = Iniciar-Transmissao $cfg.nome $cfg.comSom
        return
    }
    if (-not ($AudioPermitido -and $cfg.comSom)) { return }
    $raizes = Achar-Discord
    $mudou = if ($Script:ComSom) { -not ($raizes.Count -eq 1 -and $raizes[0] -eq $Script:PidDiscord) } else { $raizes.Count -eq 1 }
    if ($mudou) {
        # Para ANTES de qualquer coisa: com um Discord novo, o PID excluído não vale mais.
        Parar-Transmissao
        Status 'O Discord abriu ou fechou: reiniciando a transmissão…'
        $Script:AvisoSom = Iniciar-Transmissao $cfg.nome $cfg.comSom
        Status "AO VIVO como '$($cfg.nome)' ($($Script:Encoder.w)x$($Script:Encoder.h) 60fps, $($Script:Encoder.nome)).`n$($Script:AvisoSom)"
    }
})
$timer.Start()

$form.add_FormClosing({ $timer.Stop(); Parar-Transmissao })
[void]$form.ShowDialog()
