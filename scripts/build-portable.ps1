# 生成便携版 ZIP：打包 target\release\newganmanager.exe（单文件自包含）+ 使用说明.txt
# 用法：先 pnpm tauri build，再 pnpm build:portable
$ErrorActionPreference = 'Stop'

# 从 tauri.conf.json 读版本号
$conf = Get-Content "$PSScriptRoot\..\src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json
$version = $conf.version

$exe = "$PSScriptRoot\..\target\release\newganmanager.exe"
if (-not (Test-Path $exe)) {
    Write-Error "未找到 $exe——请先运行 pnpm tauri build"
}

# 临时打包目录：解压后得到一个完整的程序文件夹
$stage = "$PSScriptRoot\..\target\release\bundle\portable\NewGAN-Manager-$version"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Path $stage -Force | Out-Null
Copy-Item $exe $stage

# 生成使用说明.txt（带 BOM 的 UTF-8，记事本打开不乱码）
$readme = @"
NewGAN Manager v$version（便携版）
====================================

【首次运行】
双击 newganmanager.exe 即可运行，无需安装。

· 若 Windows 弹出蓝色警告"Windows 已保护你的电脑"：
  点「更多信息」→「仍要运行」即可。
  （本应用未购买代码签名证书，属正常提示；源码完全公开，可到 GitHub 仓库审计）
· 若杀毒软件报毒：属误报，请将本文件夹加入杀软信任区。
· 若提示缺少 WebView2：极少数精简系统会出现，到微软官网安装
  WebView2 Runtime 后即可（Win10 1809+/Win11 一般自带）。

【数据保存位置】
所有用户数据（Profile、组、映射快照、日志）保存在本文件夹的 data\ 目录里，
不写 C 盘、不留注册表。整个文件夹复制到 U 盘或别的电脑即可原样使用；
删除整个文件夹 = 彻底卸载。

【快速上手】
1. 左上角 + 新建 Profile，关联你的头像包根目录（其下是 African、Asian… 等种族子目录）
2. 配置卡中点「检查完整性」，缺目录可一键创建；再选择 FM 导出的 RTF 名单
3. 选好模式（Preserve / Overwrite / Generate）→ 点 Replace Faces
4. 回 FM：偏好设置 → 界面 → 重新加载皮肤缓存

详细说明、FM 视图（views/）与过滤器（filters/）的安装：
https://github.com/huangzhanhao/NewGAN-Manager-ByHzH
"@
[System.IO.File]::WriteAllText("$stage\使用说明.txt", $readme, (New-Object System.Text.UTF8Encoding($true)))

# 打包
$outDir = Split-Path $stage -Parent
$zip = "$outDir\NewGAN-Manager_${version}_x64-portable.zip"
Compress-Archive -Path $stage -DestinationPath $zip -Force
Remove-Item $stage -Recurse -Force

$size = [math]::Round((Get-Item $zip).Length / 1MB, 2)
Write-Host "便携版已生成：$zip（$size MB）"
