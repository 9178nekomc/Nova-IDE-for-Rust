import { readFileSync, writeFileSync } from 'node:fs';

const file = 'D:/Users/Coffee/Desktop/NovaRustIDE/src/main.rs';
let text = readFileSync(file, 'utf8');

const startMark = '    fn draw_project_dialog(&mut self, ctx: &egui::Context) {';
const endMark = '        if self.workspace.is_none() {';

const start = text.indexOf(startMark);
if (start < 0) {
  console.log('start mark not found');
  process.exit(1);
}
const end = text.indexOf(endMark, start);
if (end < 0) {
  console.log('end mark not found');
  process.exit(1);
}

const replacement = `    fn draw_project_dialog(&mut self, ctx: &egui::Context) {
        let mut dialog = match self.project_dialog.take() {
            Some(d) => d,
            None => return,
        };
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            return; // Esc 关闭
        }
        let mut cancelled = false;
        let mut confirmed = false;
        let mut change_location = false;
        let dark = self.dark_mode;
        let mut win_open = true;
        egui::Window::new("新建项目")
            .open(&mut win_open)
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("项目名称:");
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut dialog.name)
                            .desired_width(240.0)
                            .hint_text("例如: my_first_rust"),
                    );
                    if resp.lost_focus() && ctx.input(|i| i.key_pressed(Key::Enter)) {
                        confirmed = true;
                    }
                });
                ui.add_space(6.0);
                ui.label("选择模板:");
                for (idx, tname) in PROJECT_TEMPLATES.iter().enumerate() {
                    ui.radio_value(&mut dialog.template, idx, *tname);
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("创建位置:");
                    ui.label(
                        RichText::new(dialog.location.display().to_string())
                            .size(11.0)
                            .weak(),
                    );
                    if ui.small_button("更改...").clicked() {
                        change_location = true;
                    }
                });
                if let Some(err) = &dialog.error {
                    ui.label(RichText::new(err.as_str()).small().color(BRAND_RED));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("创建项目").clicked() {
                        confirmed = true;
                    }
                    if ui.button("取消").clicked() {
                        cancelled = true;
                    }
                });
            });
        if change_location {
            if let Some(dir) =
                rfd::FileDialog::new().set_title("选择新项目的创建位置").pick_folder()
            {
                dialog.location = dir;
                dialog.error = None;
            }
        }
        if cancelled {
            win_open = false;
        }
        if confirmed {
            let kept = self.create_project(dialog);
            if let Some(d) = kept {
                self.project_dialog = Some(d);
            }
        } else if win_open {
            self.project_dialog = Some(dialog);
        }
    }

    fn menu_new_file(&mut self) {
`;

text = text.slice(0, start) + replacement + text.slice(end);
writeFileSync(file, text, 'utf8');
console.log('patched: draw_project_dialog completed + menu_new_file header restored');
