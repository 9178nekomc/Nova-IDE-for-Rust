param([string]$File)
$text = [System.IO.File]::ReadAllText($File, (New-Object System.Text.UTF8Encoding($false)))
$anchor = '    fn draw_main(&mut self, ctx: &egui::Context) {'
$guard = @'
    fn draw_main(&mut self, ctx: &egui::Context) {
        // 确保窗口尺寸为 1280x800 逻辑点且与显示器 DPI 同步（防御 winit 初始 scale 竞态）
        if !self.main_size_ok {
            let native_ppp = ctx.input(|i| i.viewport().native_pixels_per_point);
            if let Some(n) = native_ppp {
                if (ctx.pixels_per_point() - n).abs() > 0.01 {
                    ctx.set_pixels_per_point(n);
                }
            }
            if let Some(rect) = ctx.input(|i| i.viewport().inner_rect) {
                let dw = (rect.width() - MAIN_SIZE.x).abs();
                let dh = (rect.height() - MAIN_SIZE.y).abs();
                if dw < 3.0 && dh < 3.0 {
                    self.main_size_ok = true;
                } else {
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(MAIN_SIZE));
                    center_window(ctx, MAIN_SIZE);
                }
            }
        }
'@
$idx = $text.IndexOf($anchor)
if ($idx -lt 0) { Write-Output 'anchor not found'; exit 1 }
if ($text.Contains('main_size_ok guard')) { Write-Output 'already patched'; exit 0 }
$text = $text.Remove($idx, $anchor.Length).Insert($idx, $guard)
[System.IO.File]::WriteAllText($File, $text, (New-Object System.Text.UTF8Encoding($false)))
Write-Output 'main_size_ok guard re-added'
