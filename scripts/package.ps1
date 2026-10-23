# 휴대용 배포 폴더(dist\TranslateB)와 zip 을 만든다.
# 엔진(OpenCode CLI, PDFium)은 공식 GitHub 릴리스에서 받아 sha256 을 확인한 뒤 넣는다.
#   powershell -ExecutionPolicy Bypass -File scripts\package.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\package.ps1 -NoEngines   (엔진 없이: 첫 실행 때 [점검] 화면에서 설치)
param([switch]$NoEngines)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'cargo build 실패' }

$dist = Join-Path $root 'dist\TranslateB'
if (Test-Path $dist) { Remove-Item -Recurse -Force $dist }
New-Item -ItemType Directory -Force $dist | Out-Null
Copy-Item 'target\release\translateb.exe' (Join-Path $dist 'TranslateB.exe')
Copy-Item -Recurse 'rules' (Join-Path $dist 'rules')
Copy-Item 'README.md' $dist

if (-not $NoEngines) {
    $env:TRANSLATEB_HOME = $dist
    # 배포용 exe 는 창 프로그램이라 & 로 부르면 끝나기를 기다리지 않는다
    $p = Start-Process -FilePath (Join-Path $dist 'TranslateB.exe') -ArgumentList '--install-engines' -NoNewWindow -Wait -PassThru
    Remove-Item Env:\TRANSLATEB_HOME
    if ($p.ExitCode -ne 0) { throw '엔진 설치 실패' }
    # 패키지 만들 때 생긴 OpenCode 데이터는 넣지 않는다
    $home2 = Join-Path $dist 'engine\home'
    if (Test-Path $home2) { Remove-Item -Recurse -Force $home2 }
}

$zip = Join-Path $root 'dist\TranslateB.zip'
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path $dist -DestinationPath $zip
Write-Host "완료: $zip"
