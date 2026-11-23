# Junta tudo que o instalador leva: o app, o GStreamer (bibliotecas e só os
# plugins usados), o servidor de vídeo e o runtime do Visual C++.
param(
    [string]$Gst = "C:\gstreamer",
    [string]$Out = "desktop\installer\bundle",
    [string]$MediaMtxVersion = "v1.21.4"
)
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

if (Test-Path $Out) { Remove-Item -Recurse -Force $Out }
New-Item -ItemType Directory -Force "$Out\gstreamer-1.0" | Out-Null

Copy-Item "desktop\target\release\sala-de-tela.exe" $Out
Copy-Item "$Gst\bin\*.dll" $Out
Copy-Item "$Gst\libexec\gstreamer-1.0\gst-plugin-scanner.exe" $Out

$plugins = @(
    "coreelements", "app", "videoconvertscale", "videorate", "videotestsrc", "audioconvert",
    "audioresample", "audiotestsrc", "volume", "autodetect", "opus", "opusparse", "srt",
    "mpegtsmux", "mpegtsdemux", "videoparsersbad", "d3d11", "nvcodec", "amfcodec", "qsv",
    "mediafoundation", "wasapi2", "directsound", "typefindfunctions", "libav"
)
foreach ($p in $plugins) {
    $dll = "$Gst\lib\gstreamer-1.0\gst$p.dll"
    if (-not (Test-Path $dll)) { throw "plugin ausente no GStreamer: $p" }
    Copy-Item $dll "$Out\gstreamer-1.0"
}

foreach ($dll in "vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll") {
    Copy-Item "$env:WINDIR\System32\$dll" $Out
}

$zip = "mediamtx_${MediaMtxVersion}_windows_amd64.zip"
$base = "https://github.com/bluenviron/mediamtx/releases/download/$MediaMtxVersion"
Invoke-WebRequest "$base/$zip" -OutFile $zip
Invoke-WebRequest "$base/checksums.sha256" -OutFile checksums.sha256
$line = Get-Content checksums.sha256 | Where-Object { $_ -match [regex]::Escape($zip) }
$expected = ($line -split "\s+")[0].ToLower()
$actual = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
if ($expected -ne $actual) { throw "checksum do MediaMTX não confere" }
Expand-Archive $zip -DestinationPath mediamtx-tmp -Force
Copy-Item "mediamtx-tmp\mediamtx.exe" $Out

# Confere que cada elemento que o app usa carrega a partir do pacote. Fora da
# lista: codificadores e decodificadores de hardware (d3d11h264dec, nvcodec,
# amf, qsv), que o GStreamer só registra se a placa de vídeo os suporta; o
# runner do CI não tem GPU. O avdec_h264 garante a decodificação sem eles.
$env:GST_PLUGIN_SYSTEM_PATH_1_0 = (Resolve-Path "$Out\gstreamer-1.0").Path
$env:GST_PLUGIN_PATH_1_0 = ""
$env:GST_REGISTRY_1_0 = "$env:TEMP\bundle-registry.bin"
$elements = @(
    "d3d11screencapturesrc", "d3d11testsrc", "d3d11convert", "d3d11download", "avdec_h264", "wasapi2src",
    "wasapi2sink", "srtsrc", "srtsink", "mpegtsmux", "tsdemux", "h264parse", "opusenc", "opusdec", "opusparse",
    "appsink", "videoconvertscale", "audioconvert", "audioresample", "volume", "autoaudiosink", "mfh264enc"
)
$missing = @()
foreach ($e in $elements) {
    & "$Gst\bin\gst-inspect-1.0.exe" --exists $e
    if ($LASTEXITCODE -ne 0) { $missing += $e }
}
if ($missing.Count -gt 0) { throw "elementos que não carregaram do pacote: $($missing -join ', ')" }

$size = (Get-ChildItem -Recurse $Out | Measure-Object -Property Length -Sum).Sum / 1MB
Write-Host ("Pacote pronto: {0:N0} MB" -f $size)
