# Sala de Tela: instalador automático para Windows 10/11.
# NÃO rode este arquivo direto: o build-instalador.sh embute ele no
# Instalar-Sala.bat. O instalador é genérico (pode ser público): o IP do
# servidor e a chave da VPN chegam pelo "código de convite", que o dono da
# Sala manda em mensagem privada (gerado pelo gerar-convite.sh).
# Roda como administrador (o .bat pede o "Sim" do UAC).

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName Microsoft.VisualBasic

# Preenchido pelo build-instalador.sh
$TransmitirB64 = '__TRANSMITIR_B64__'

$GstVersao = '1.28.8'
$GstArq = "gstreamer-1.0-msvc-x86_64-$GstVersao.exe"
$GstUrl = "https://gstreamer.freedesktop.org/data/pkg/windows/$GstVersao/msvc/$GstArq"
$TsUrl = 'https://pkgs.tailscale.com/stable/tailscale-setup-latest-amd64.msi'

$Pasta = Join-Path $env:ProgramData 'SalaDeTela'
$Tmp = Join-Path $env:TEMP 'SalaDeTela-instalador'
$Log = Join-Path $Pasta 'instalacao.log'
New-Item -ItemType Directory -Force -Path $Pasta, $Tmp | Out-Null
Start-Transcript -Path $Log -Force | Out-Null

function Passo([string]$t) { Write-Host ''; Write-Host "==> $t" -ForegroundColor Cyan }
function Ok([string]$t) { Write-Host "    OK: $t" -ForegroundColor Green }

function Falhar([string]$t) {
    Write-Host "ERRO: $t" -ForegroundColor Red
    [System.Windows.Forms.MessageBox]::Show(
        "Algo deu errado:`n`n$t`n`nMande o arquivo $Log para o dono da Sala.",
        'Sala de Tela', 'OK', 'Error') | Out-Null
    Stop-Transcript | Out-Null
    exit 1
}

# Roda um programa externo sem que mensagens no stderr virem erro fatal
# (no PowerShell 5.1, "2>&1" com ErrorActionPreference=Stop derruba o script).
function Nativo {
    $ErrorActionPreference = 'Continue'
    $exe, $resto = $args
    & $exe @resto 2>&1 | ForEach-Object { Write-Host "    $_" }
}

function Baixar([string]$url, [string]$destino) {
    # curl.exe vem com o Windows 10/11 e é bem mais rápido que o Invoke-WebRequest
    & curl.exe -L --fail --retry 3 --progress-bar -o $destino $url
    if ($LASTEXITCODE -ne 0) { Falhar "Não consegui baixar $url (sem internet?)" }
}

# Código de convite: "SALA-" + base64("ip|authkey|audio").
function Ler-Convite([string]$codigo) {
    $codigo = ($codigo -replace '\s', '')
    if (-not $codigo.StartsWith('SALA-')) { return $null }
    try {
        $partes = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($codigo.Substring(5))).Split('|')
    } catch { return $null }
    if ($partes.Count -ne 3 -or $partes[0] -notmatch '^100\.\d+\.\d+\.\d+$' -or
        $partes[1] -notmatch '^tskey-auth-[A-Za-z0-9_-]+$' -or $partes[2] -notin 'true', 'false') { return $null }
    return [pscustomobject]@{ ip = $partes[0]; chave = $partes[1]; audio = $partes[2] }
}

# Reinstalação: se já está na VPN da Sala, não precisa de convite de novo.
$ArqServidor = Join-Path $Pasta 'servidor.json'
$Convite = $null
$JaConectado = $false
if (Test-Path $ArqServidor) {
    $Convite = Get-Content $ArqServidor -Raw | ConvertFrom-Json
    try { Invoke-WebRequest -UseBasicParsing -TimeoutSec 5 "http://$($Convite.ip):8080/api/ao-vivo" | Out-Null; $JaConectado = $true }
    catch { $Convite = $null }
}
while (-not $Convite) {
    $codigo = [Microsoft.VisualBasic.Interaction]::InputBox(
        "Cole aqui o código de convite que você recebeu (começa com SALA-):", 'Sala de Tela', '')
    if (-not $codigo) { Stop-Transcript | Out-Null; exit 0 }
    $Convite = Ler-Convite $codigo
    if (-not $Convite) {
        [System.Windows.Forms.MessageBox]::Show('Esse código não é válido. Copie de novo a mensagem inteira, começando em SALA-.',
            'Sala de Tela', 'OK', 'Warning') | Out-Null
    }
}
$Servidor = $Convite.ip

Write-Host '=============================================' -ForegroundColor Yellow
Write-Host '  Instalando a Sala de Tela. Pode demorar uns' -ForegroundColor Yellow
Write-Host '  minutos. NÃO feche esta janela.' -ForegroundColor Yellow
Write-Host '=============================================' -ForegroundColor Yellow

try {
    # ------------------------------------------------------------ Windows
    $build = [int](Get-CimInstance Win32_OperatingSystem).BuildNumber
    if ($build -lt 20348) {
        Write-Host "    Aviso: Windows build ${build}: a transmissão vai funcionar SEM som (precisa do Windows 11)." -ForegroundColor Yellow
    }

    # ------------------------------------------------------------ Tailscale
    Passo 'VPN (Tailscale)'
    $ts = Join-Path $env:ProgramFiles 'Tailscale\tailscale.exe'
    if (-not (Test-Path $ts)) {
        $msi = Join-Path $Tmp 'tailscale.msi'
        Baixar $TsUrl $msi
        $p = Start-Process msiexec.exe -Wait -PassThru -ArgumentList "/i `"$msi`" /quiet /norestart TS_UNATTENDEDMODE=always TS_NOLAUNCH=1"
        if ($p.ExitCode -notin 0, 3010) { Falhar "O instalador do Tailscale terminou com código $($p.ExitCode)." }
    }
    for ($i = 0; $i -lt 30 -and -not (Test-Path $ts); $i++) { Start-Sleep 1 }
    for ($i = 0; $i -lt 30 -and (Get-Service Tailscale -ErrorAction SilentlyContinue).Status -ne 'Running'; $i++) { Start-Sleep 1 }

    if (-not $JaConectado) {
        $AuthKey = $Convite.chave
        $hostname = 'sala-' + (($env:COMPUTERNAME.ToLowerInvariant()) -replace '[^a-z0-9-]', '-')
        Nativo $ts 'up' "--auth-key=$AuthKey" '--unattended' "--hostname=$hostname" '--accept-routes=false'
        if ($LASTEXITCODE -ne 0) {
            # Já estava logado em outra rede: força entrar na rede da Sala.
            Nativo $ts 'up' '--force-reauth' "--auth-key=$AuthKey" '--unattended' "--hostname=$hostname" '--accept-routes=false'
            if ($LASTEXITCODE -ne 0) { Falhar 'Não consegui entrar na VPN. O código de convite já foi usado ou expirou: peça um novo.' }
        }
    }
    # Guarda só o IP e a opção de áudio; a chave (uso único) não fica salva.
    [pscustomobject]@{ ip = $Convite.ip; audio = $Convite.audio } | ConvertTo-Json | Set-Content $ArqServidor -Encoding UTF8
    Ok "conectado à VPN ($(& $ts ip -4))"

    # ------------------------------------------------------------ GStreamer
    Passo "GStreamer $GstVersao (captura e envio da tela)"
    $gstRaiz = Join-Path $env:ProgramFiles 'gstreamer\1.0\msvc_x86_64'
    $gstLaunch = Join-Path $gstRaiz 'bin\gst-launch-1.0.exe'
    $versaoAtual = if (Test-Path $gstLaunch) { (& $gstLaunch --version | Select-Object -First 1) } else { '' }
    if ($versaoAtual -notmatch [regex]::Escape($GstVersao)) {
        Write-Host '    Baixando (~540 MB, só na primeira vez)...'
        $exe = Join-Path $Tmp $GstArq
        $sha = Join-Path $Tmp "$GstArq.sha256sum"
        Baixar "$GstUrl.sha256sum" $sha
        $esperado = ((Get-Content $sha -Raw).Trim() -split '\s+')[0].ToLowerInvariant()
        $precisaBaixar = $true
        if (Test-Path $exe) { $precisaBaixar = ((Get-FileHash $exe -Algorithm SHA256).Hash.ToLowerInvariant() -ne $esperado) }
        if ($precisaBaixar) { Baixar $GstUrl $exe }
        $obtido = (Get-FileHash $exe -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($obtido -ne $esperado) { Remove-Item $exe -Force; Falhar 'O download do GStreamer veio corrompido. Rode o instalador de novo.' }
        Write-Host '    Instalando...'
        $p = Start-Process $exe -Wait -PassThru -ArgumentList '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- /ALLUSERS /TYPE=runtime'
        if ($p.ExitCode -ne 0) { Falhar "O instalador do GStreamer terminou com código $($p.ExitCode)." }
        Remove-Item $exe -Force -ErrorAction SilentlyContinue
    }
    if (-not (Test-Path $gstLaunch)) { Falhar "GStreamer não apareceu em $gstRaiz." }
    Ok (& $gstLaunch --version | Select-Object -First 1)

    # ------------------------------------------------------------ transmissor
    Passo 'Transmissor e atalhos'
    $transmitir = Join-Path $Pasta 'Transmitir.ps1'
    $bytes = [Convert]::FromBase64String($TransmitirB64)
    $texto = [Text.Encoding]::UTF8.GetString($bytes).Replace('__IP_SERVIDOR__', $Servidor).Replace('__AUDIO_PERMITIDO__', $Convite.audio)
    # Com BOM: o PowerShell 5.1 lê acentos errado sem ele.
    [IO.File]::WriteAllText($transmitir, $texto, (New-Object Text.UTF8Encoding $true))

    $desktop = [Environment]::GetFolderPath('CommonDesktopDirectory')
    $shell = New-Object -ComObject WScript.Shell

    $lnk = $shell.CreateShortcut((Join-Path $desktop 'Transmitir minha tela.lnk'))
    $lnk.TargetPath = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $lnk.Arguments = "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$transmitir`""
    $lnk.IconLocation = (Join-Path $env:SystemRoot 'System32\shell32.dll') + ',17'
    $lnk.Description = 'Transmitir a sua tela para a Sala'
    $lnk.Save()

    $edge = Join-Path ${env:ProgramFiles(x86)} 'Microsoft\Edge\Application\msedge.exe'
    $lnk = $shell.CreateShortcut((Join-Path $desktop 'Sala de Tela.lnk'))
    if (Test-Path $edge) {
        $lnk.TargetPath = $edge
        $lnk.Arguments = "--app=http://${Servidor}:8080/"
        $lnk.IconLocation = "$edge,0"
    } else {
        $lnk.TargetPath = "http://${Servidor}:8080/"
    }
    $lnk.Description = 'Assistir a tela dos amigos'
    $lnk.Save()
    Ok 'atalhos "Sala de Tela" e "Transmitir minha tela" na Área de Trabalho'

    # Monta o cache de plugins do GStreamer agora (a primeira vez é lenta).
    Passo 'Preparando o GStreamer (primeira vez)'
    Nativo (Join-Path $gstRaiz 'bin\gst-inspect-1.0.exe') '--exists' 'whipclientsink'
    if ($LASTEXITCODE -ne 0) { Write-Host '    Aviso: whipclientsink não encontrado (o Diagnóstico vai mostrar).' -ForegroundColor Yellow }

    # ------------------------------------------------------------ teste
    Passo 'Testando a conexão com a Sala'
    $alcancou = $false
    for ($i = 0; $i -lt 10 -and -not $alcancou; $i++) {
        try { Invoke-WebRequest -UseBasicParsing -TimeoutSec 5 "http://${Servidor}:8080/api/ao-vivo" | Out-Null; $alcancou = $true }
        catch { Start-Sleep 2 }
    }
    Remove-Item $Tmp -Recurse -Force -ErrorAction SilentlyContinue
    Stop-Transcript | Out-Null

    $msg = "Pronto! Instalação concluída.`n`n" +
           "Na Área de Trabalho agora tem:`n" +
           "  • Sala de Tela: para assistir`n" +
           "  • Transmitir minha tela: para mostrar a sua tela`n`n" +
           "Use o app do Discord (não o Discord no navegador) para a voz não vazar na transmissão."
    if (-not $alcancou) {
        $msg += "`n`nObs.: não consegui falar com o PC do servidor agora. Ele está ligado? Se estiver, tente de novo em alguns minutos."
    }
    [System.Windows.Forms.MessageBox]::Show($msg, 'Sala de Tela', 'OK', 'Information') | Out-Null
} catch {
    Falhar $_.Exception.Message
}
