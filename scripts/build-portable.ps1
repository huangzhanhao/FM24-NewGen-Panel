# 生成便携版 ZIP：打包 target\release\newganmanager.exe（单文件自包含）
# 用法：先 pnpm tauri build，再 scripts\build-portable.ps1
$ErrorActionPreference = 'Stop'

# 从 tauri.conf.json 读版本号
$conf = Get-Content "$PSScriptRoot\..\src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json
$version = $conf.version

$exe = "$PSScriptRoot\..\target\release\newganmanager.exe"
if (-not (Test-Path $exe)) {
    Write-Error "未找到 $exe——请先运行 pnpm tauri build"
}

$outDir = "$PSScriptRoot\..\target\release\bundle\portable"
New-Item -ItemType Directory -Path $outDir -Force | Out-Null
$zip = "$outDir\NewGAN-Manager_${version}_x64-portable.zip"

Compress-Archive -Path $exe -DestinationPath $zip -Force
$size = [math]::Round((Get-Item $zip).Length / 1MB, 2)
Write-Host "便携版已生成：$zip（$size MB）"
