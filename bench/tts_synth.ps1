# Synthesises benchmark sentences with the Windows OneCore TTS voices (WinRT), one WAV per job.
# Usage: powershell -NoProfile -File tts_synth.ps1 -Jobs jobs.json
# jobs.json: [{"voice":"Heera","rate":1.0,"text":"...","out":"C:\\...\\x.wav"}, ...]
# Output is whatever the voice produces (converted to 16 kHz mono by make_tts_corpus.py via ffmpeg).
param([Parameter(Mandatory = $true)][string]$Jobs)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Runtime.WindowsRuntime
[Windows.Media.SpeechSynthesis.SpeechSynthesizer, Windows.Media.SpeechSynthesis, ContentType = WindowsRuntime] | Out-Null
[Windows.Storage.Streams.DataReader, Windows.Storage.Streams, ContentType = WindowsRuntime] | Out-Null
$asTask = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
        $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' })[0]
function Await($op, $type) {
    $t = $asTask.MakeGenericMethod($type).Invoke($null, @($op)); $t.Wait(-1) | Out-Null; $t.Result
}
$all = [Windows.Media.SpeechSynthesis.SpeechSynthesizer]::AllVoices
$synth = New-Object Windows.Media.SpeechSynthesis.SpeechSynthesizer
$list = Get-Content -Raw -Encoding UTF8 $Jobs | ConvertFrom-Json
$done = 0
foreach ($j in $list) {
    if (Test-Path $j.out) { continue }
    $v = $all | Where-Object { $_.DisplayName -match $j.voice } | Select-Object -First 1
    if (-not $v) { throw "voice not installed: $($j.voice). Installed: $(($all | ForEach-Object DisplayName) -join ', ')" }
    $synth.Voice = $v
    $synth.Options.SpeakingRate = [double]$j.rate
    $stream = Await ($synth.SynthesizeTextToStreamAsync($j.text)) ([Windows.Media.SpeechSynthesis.SpeechSynthesisStream])
    $reader = New-Object Windows.Storage.Streams.DataReader($stream.GetInputStreamAt(0))
    Await ($reader.LoadAsync([uint32]$stream.Size)) ([uint32]) | Out-Null
    $bytes = New-Object byte[] ([int]$stream.Size)
    $reader.ReadBytes($bytes)
    [IO.File]::WriteAllBytes($j.out, $bytes)
    $done++
}
Write-Output "synthesised $done new file(s); voices: $(($all | ForEach-Object DisplayName) -join ', ')"
