// NovaRS — Multi-Theme Engine (Code Highlighting & UI Chrome Themes)
// Supports: VS Code Dark+, One Dark Pro, Dracula, Tokyo Night, Catppuccin Mocha,
// Monokai Pro, Solarized Light, GitHub Light

use eframe::egui::{self, Color32, Visuals};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ThemePreset {
    VsCodeDarkPlus,
    OneDarkPro,
    Dracula,
    TokyoNight,
    CatppuccinMocha,
    MonokaiPro,
    SolarizedLight,
    GitHubLight,
}

impl Default for ThemePreset {
    fn default() -> Self {
        ThemePreset::VsCodeDarkPlus
    }
}

impl ThemePreset {
    pub const ALL: &'static [ThemePreset] = &[
        ThemePreset::VsCodeDarkPlus,
        ThemePreset::OneDarkPro,
        ThemePreset::Dracula,
        ThemePreset::TokyoNight,
        ThemePreset::CatppuccinMocha,
        ThemePreset::MonokaiPro,
        ThemePreset::SolarizedLight,
        ThemePreset::GitHubLight,
    ];

    pub fn display_name(&self) -> &'static str {
        match self {
            ThemePreset::VsCodeDarkPlus => "VS Code Dark+",
            ThemePreset::OneDarkPro => "One Dark Pro",
            ThemePreset::Dracula => "Dracula",
            ThemePreset::TokyoNight => "Tokyo Night",
            ThemePreset::CatppuccinMocha => "Catppuccin Mocha",
            ThemePreset::MonokaiPro => "Monokai Pro",
            ThemePreset::SolarizedLight => "Solarized Light",
            ThemePreset::GitHubLight => "GitHub Light",
        }
    }

    pub fn is_dark(&self) -> bool {
        !matches!(self, ThemePreset::SolarizedLight | ThemePreset::GitHubLight)
    }

    pub fn apply_to_context(&self, ctx: &egui::Context) {
        let palette = CodeTheme::from_preset(*self);
        let mut visuals = if self.is_dark() {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        visuals.panel_fill = palette.panel_bg;
        visuals.window_fill = palette.panel_bg;
        visuals.extreme_bg_color = palette.editor_bg;
        visuals.faint_bg_color = palette.gutter_bg;
        visuals.code_bg_color = palette.editor_bg;
        visuals.selection.bg_fill = palette.selection_bg;
        ctx.set_visuals(visuals);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TokKind {
    Keyword,
    ControlFlow,
    Str,
    Comment,
    Number,
    Ty,
    Function,
    Macro,
    Lifetime,
    Attribute,
    Operator,
    Normal,
}

#[derive(Clone)]
pub struct CodeTheme {
    pub preset: ThemePreset,
    pub dark: bool,
    pub editor_bg: Color32,
    pub panel_bg: Color32,
    pub gutter_bg: Color32,
    pub current_line_bg: Color32,
    pub selection_bg: Color32,
    pub keyword: Color32,
    pub control_flow: Color32,
    pub string: Color32,
    pub comment: Color32,
    pub number: Color32,
    pub ty: Color32,
    pub function: Color32,
    pub macro_call: Color32,
    pub lifetime: Color32,
    pub attribute: Color32,
    pub operator: Color32,
    pub normal: Color32,
    pub line_number: Color32,
    pub line_number_active: Color32,
    pub indent_guide: Color32,
    pub indent_guide_active: Color32,
    pub error_squiggly: Color32,
    pub warning_squiggly: Color32,
    pub info_squiggly: Color32,
}

impl CodeTheme {
    pub fn new(dark: bool) -> Self {
        if dark {
            Self::from_preset(ThemePreset::VsCodeDarkPlus)
        } else {
            Self::from_preset(ThemePreset::GitHubLight)
        }
    }

    pub fn from_preset(preset: ThemePreset) -> Self {
        match preset {
            ThemePreset::VsCodeDarkPlus => CodeTheme {
                preset,
                dark: true,
                editor_bg: Color32::from_rgb(0x1e, 0x1e, 0x1e),
                panel_bg: Color32::from_rgb(0x25, 0x25, 0x26),
                gutter_bg: Color32::from_rgb(0x1e, 0x1e, 0x1e),
                current_line_bg: Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 10),
                selection_bg: Color32::from_rgba_unmultiplied(0x26, 0x4f, 0x78, 180),
                keyword: Color32::from_rgb(0x56, 0x9c, 0xd6),
                control_flow: Color32::from_rgb(0xc5, 0x86, 0xc0),
                string: Color32::from_rgb(0xce, 0x91, 0x78),
                comment: Color32::from_rgb(0x6a, 0x99, 0x55),
                number: Color32::from_rgb(0xb5, 0xce, 0xa8),
                ty: Color32::from_rgb(0x4e, 0xc9, 0xb0),
                function: Color32::from_rgb(0xdc, 0xdc, 0xaa),
                macro_call: Color32::from_rgb(0x56, 0x9c, 0xd6),
                lifetime: Color32::from_rgb(0xd7, 0xba, 0x7d),
                attribute: Color32::from_rgb(0x9c, 0xdc, 0xfe),
                operator: Color32::from_rgb(0xd4, 0xd4, 0xd4),
                normal: Color32::from_rgb(0xd4, 0xd4, 0xd4),
                line_number: Color32::from_rgb(0x85, 0x85, 0x85),
                line_number_active: Color32::from_rgb(0xc6, 0xc6, 0xc6),
                indent_guide: Color32::from_rgba_unmultiplied(0x40, 0x40, 0x40, 175),
                indent_guide_active: Color32::from_rgba_unmultiplied(0x70, 0x70, 0x70, 235),
                error_squiggly: Color32::from_rgb(0xf1, 0x4c, 0x4c),
                warning_squiggly: Color32::from_rgb(0xcc, 0xa7, 0x00),
                info_squiggly: Color32::from_rgb(0x37, 0x94, 0xff),
            },
            ThemePreset::OneDarkPro => CodeTheme {
                preset,
                dark: true,
                editor_bg: Color32::from_rgb(0x28, 0x2c, 0x34),
                panel_bg: Color32::from_rgb(0x21, 0x25, 0x2b),
                gutter_bg: Color32::from_rgb(0x28, 0x2c, 0x34),
                current_line_bg: Color32::from_rgba_unmultiplied(0x2c, 0x31, 0x3c, 160),
                selection_bg: Color32::from_rgba_unmultiplied(0x3e, 0x44, 0x51, 200),
                keyword: Color32::from_rgb(0xc6, 0x78, 0xdd),
                control_flow: Color32::from_rgb(0xc6, 0x78, 0xdd),
                string: Color32::from_rgb(0x98, 0xc3, 0x79),
                comment: Color32::from_rgb(0x5c, 0x63, 0x70),
                number: Color32::from_rgb(0xd1, 0x9a, 0x66),
                ty: Color32::from_rgb(0xe5, 0xc0, 0x7b),
                function: Color32::from_rgb(0x61, 0xaf, 0xef),
                macro_call: Color32::from_rgb(0x56, 0xb6, 0xc2),
                lifetime: Color32::from_rgb(0xe0, 0x6c, 0x75),
                attribute: Color32::from_rgb(0xd1, 0x9a, 0x66),
                operator: Color32::from_rgb(0x56, 0xb6, 0xc2),
                normal: Color32::from_rgb(0xab, 0xb2, 0xbf),
                line_number: Color32::from_rgb(0x4b, 0x52, 0x63),
                line_number_active: Color32::from_rgb(0xab, 0xb2, 0xbf),
                indent_guide: Color32::from_rgba_unmultiplied(0x3b, 0x40, 0x48, 180),
                indent_guide_active: Color32::from_rgba_unmultiplied(0x7f, 0x84, 0x8e, 230),
                error_squiggly: Color32::from_rgb(0xe0, 0x6c, 0x75),
                warning_squiggly: Color32::from_rgb(0xe5, 0xc0, 0x7b),
                info_squiggly: Color32::from_rgb(0x61, 0xaf, 0xef),
            },
            ThemePreset::Dracula => CodeTheme {
                preset,
                dark: true,
                editor_bg: Color32::from_rgb(0x28, 0x2a, 0x36),
                panel_bg: Color32::from_rgb(0x21, 0x22, 0x2c),
                gutter_bg: Color32::from_rgb(0x28, 0x2a, 0x36),
                current_line_bg: Color32::from_rgba_unmultiplied(0x44, 0x47, 0x5a, 90),
                selection_bg: Color32::from_rgba_unmultiplied(0x44, 0x47, 0x5a, 200),
                keyword: Color32::from_rgb(0xff, 0x79, 0xc6),
                control_flow: Color32::from_rgb(0xff, 0x79, 0xc6),
                string: Color32::from_rgb(0xf1, 0xfa, 0x8c),
                comment: Color32::from_rgb(0x62, 0x72, 0xa4),
                number: Color32::from_rgb(0xbd, 0x93, 0xf9),
                ty: Color32::from_rgb(0x8b, 0xe9, 0xfd),
                function: Color32::from_rgb(0x50, 0xfa, 0x7b),
                macro_call: Color32::from_rgb(0x8b, 0xe9, 0xfd),
                lifetime: Color32::from_rgb(0xff, 0xb8, 0x6c),
                attribute: Color32::from_rgb(0x50, 0xfa, 0x7b),
                operator: Color32::from_rgb(0xff, 0x79, 0xc6),
                normal: Color32::from_rgb(0xf8, 0xf8, 0xf2),
                line_number: Color32::from_rgb(0x62, 0x72, 0xa4),
                line_number_active: Color32::from_rgb(0xf8, 0xf8, 0xf2),
                indent_guide: Color32::from_rgba_unmultiplied(0x44, 0x47, 0x5a, 165),
                indent_guide_active: Color32::from_rgba_unmultiplied(0xbd, 0x93, 0xf9, 220),
                error_squiggly: Color32::from_rgb(0xff, 0x55, 0x55),
                warning_squiggly: Color32::from_rgb(0xff, 0xb8, 0x6c),
                info_squiggly: Color32::from_rgb(0x8b, 0xe9, 0xfd),
            },
            ThemePreset::TokyoNight => CodeTheme {
                preset,
                dark: true,
                editor_bg: Color32::from_rgb(0x1a, 0x1b, 0x26),
                panel_bg: Color32::from_rgb(0x16, 0x16, 0x1e),
                gutter_bg: Color32::from_rgb(0x1a, 0x1b, 0x26),
                current_line_bg: Color32::from_rgba_unmultiplied(0x29, 0x2e, 0x42, 140),
                selection_bg: Color32::from_rgba_unmultiplied(0x33, 0x46, 0x7c, 190),
                keyword: Color32::from_rgb(0xbb, 0x9a, 0xf7),
                control_flow: Color32::from_rgb(0x9d, 0x7c, 0xd8),
                string: Color32::from_rgb(0x9e, 0xce, 0x6a),
                comment: Color32::from_rgb(0x56, 0x5f, 0x89),
                number: Color32::from_rgb(0xff, 0x9e, 0x64),
                ty: Color32::from_rgb(0x2a, 0xc3, 0xde),
                function: Color32::from_rgb(0x7a, 0xa2, 0xf7),
                macro_call: Color32::from_rgb(0x7d, 0xcf, 0xff),
                lifetime: Color32::from_rgb(0xf7, 0x76, 0x8e),
                attribute: Color32::from_rgb(0xe0, 0xaf, 0x68),
                operator: Color32::from_rgb(0x89, 0xdd, 0xff),
                normal: Color32::from_rgb(0xc0, 0xca, 0xf5),
                line_number: Color32::from_rgb(0x3b, 0x42, 0x61),
                line_number_active: Color32::from_rgb(0xa9, 0xb1, 0xd6),
                indent_guide: Color32::from_rgba_unmultiplied(0x29, 0x2e, 0x42, 185),
                indent_guide_active: Color32::from_rgba_unmultiplied(0x7a, 0xa2, 0xf7, 210),
                error_squiggly: Color32::from_rgb(0xf7, 0x76, 0x8e),
                warning_squiggly: Color32::from_rgb(0xe0, 0xaf, 0x68),
                info_squiggly: Color32::from_rgb(0x7a, 0xa2, 0xf7),
            },
            ThemePreset::CatppuccinMocha => CodeTheme {
                preset,
                dark: true,
                editor_bg: Color32::from_rgb(0x1e, 0x1e, 0x2e),
                panel_bg: Color32::from_rgb(0x18, 0x18, 0x25),
                gutter_bg: Color32::from_rgb(0x1e, 0x1e, 0x2e),
                current_line_bg: Color32::from_rgba_unmultiplied(0x31, 0x32, 0x44, 140),
                selection_bg: Color32::from_rgba_unmultiplied(0x45, 0x47, 0x5a, 190),
                keyword: Color32::from_rgb(0xcb, 0xa6, 0xf7),
                control_flow: Color32::from_rgb(0xcb, 0xa6, 0xf7),
                string: Color32::from_rgb(0xa6, 0xe3, 0xa1),
                comment: Color32::from_rgb(0x6c, 0x70, 0x86),
                number: Color32::from_rgb(0xfa, 0xb3, 0x87),
                ty: Color32::from_rgb(0xf9, 0xe2, 0xaf),
                function: Color32::from_rgb(0x89, 0xb4, 0xfa),
                macro_call: Color32::from_rgb(0x94, 0xe2, 0xd5),
                lifetime: Color32::from_rgb(0xf3, 0x8b, 0xa8),
                attribute: Color32::from_rgb(0xf5, 0xc2, 0xe7),
                operator: Color32::from_rgb(0x89, 0xdc, 0xeb),
                normal: Color32::from_rgb(0xcd, 0xd6, 0xf4),
                line_number: Color32::from_rgb(0x58, 0x5b, 0x70),
                line_number_active: Color32::from_rgb(0xcd, 0xd6, 0xf4),
                indent_guide: Color32::from_rgba_unmultiplied(0x31, 0x32, 0x44, 180),
                indent_guide_active: Color32::from_rgba_unmultiplied(0xcb, 0xa6, 0xf7, 210),
                error_squiggly: Color32::from_rgb(0xf3, 0x8b, 0xa8),
                warning_squiggly: Color32::from_rgb(0xf9, 0xe2, 0xaf),
                info_squiggly: Color32::from_rgb(0x89, 0xb4, 0xfa),
            },
            ThemePreset::MonokaiPro => CodeTheme {
                preset,
                dark: true,
                editor_bg: Color32::from_rgb(0x2d, 0x2a, 0x2e),
                panel_bg: Color32::from_rgb(0x22, 0x1f, 0x22),
                gutter_bg: Color32::from_rgb(0x2d, 0x2a, 0x2e),
                current_line_bg: Color32::from_rgba_unmultiplied(0x40, 0x3e, 0x41, 130),
                selection_bg: Color32::from_rgba_unmultiplied(0x5b, 0x59, 0x5c, 180),
                keyword: Color32::from_rgb(0xff, 0x61, 0x88),
                control_flow: Color32::from_rgb(0xff, 0x61, 0x88),
                string: Color32::from_rgb(0xff, 0xd8, 0x66),
                comment: Color32::from_rgb(0x72, 0x70, 0x72),
                number: Color32::from_rgb(0xab, 0x9d, 0xf2),
                ty: Color32::from_rgb(0x78, 0xdc, 0xe8),
                function: Color32::from_rgb(0xa9, 0xdc, 0x76),
                macro_call: Color32::from_rgb(0x78, 0xdc, 0xe8),
                lifetime: Color32::from_rgb(0xfc, 0x98, 0x67),
                attribute: Color32::from_rgb(0xab, 0x9d, 0xf2),
                operator: Color32::from_rgb(0xff, 0x61, 0x88),
                normal: Color32::from_rgb(0xfc, 0xfc, 0xfa),
                line_number: Color32::from_rgb(0x5b, 0x59, 0x5c),
                line_number_active: Color32::from_rgb(0xfc, 0xfc, 0xfa),
                indent_guide: Color32::from_rgba_unmultiplied(0x40, 0x3e, 0x41, 180),
                indent_guide_active: Color32::from_rgba_unmultiplied(0xff, 0xd8, 0x66, 200),
                error_squiggly: Color32::from_rgb(0xff, 0x61, 0x88),
                warning_squiggly: Color32::from_rgb(0xfc, 0x98, 0x67),
                info_squiggly: Color32::from_rgb(0x78, 0xdc, 0xe8),
            },
            ThemePreset::SolarizedLight => CodeTheme {
                preset,
                dark: false,
                editor_bg: Color32::from_rgb(0xfd, 0xf6, 0xe3),
                panel_bg: Color32::from_rgb(0xee, 0xe8, 0xd5),
                gutter_bg: Color32::from_rgb(0xfd, 0xf6, 0xe3),
                current_line_bg: Color32::from_rgba_unmultiplied(0xee, 0xe8, 0xd5, 180),
                selection_bg: Color32::from_rgba_unmultiplied(0xd3, 0xcb, 0xb7, 190),
                keyword: Color32::from_rgb(0x85, 0x99, 0x00),
                control_flow: Color32::from_rgb(0xd3, 0x36, 0x82),
                string: Color32::from_rgb(0x2a, 0xa1, 0x98),
                comment: Color32::from_rgb(0x93, 0xa1, 0xa1),
                number: Color32::from_rgb(0xd3, 0x36, 0x82),
                ty: Color32::from_rgb(0xb5, 0x89, 0x00),
                function: Color32::from_rgb(0x26, 0x8b, 0xd2),
                macro_call: Color32::from_rgb(0xcb, 0x4b, 0x16),
                lifetime: Color32::from_rgb(0xdc, 0x32, 0x2f),
                attribute: Color32::from_rgb(0x6c, 0x71, 0xc4),
                operator: Color32::from_rgb(0x65, 0x7b, 0x83),
                normal: Color32::from_rgb(0x58, 0x6e, 0x75),
                line_number: Color32::from_rgb(0x93, 0xa1, 0xa1),
                line_number_active: Color32::from_rgb(0x07, 0x36, 0x42),
                indent_guide: Color32::from_rgba_unmultiplied(0x93, 0xa1, 0xa1, 90),
                indent_guide_active: Color32::from_rgba_unmultiplied(0x26, 0x8b, 0xd2, 190),
                error_squiggly: Color32::from_rgb(0xdc, 0x32, 0x2f),
                warning_squiggly: Color32::from_rgb(0xb5, 0x89, 0x00),
                info_squiggly: Color32::from_rgb(0x26, 0x8b, 0xd2),
            },
            ThemePreset::GitHubLight => CodeTheme {
                preset,
                dark: false,
                editor_bg: Color32::from_rgb(0xff, 0xff, 0xff),
                panel_bg: Color32::from_rgb(0xf6, 0xf8, 0xfa),
                gutter_bg: Color32::from_rgb(0xff, 0xff, 0xff),
                current_line_bg: Color32::from_rgba_unmultiplied(0xea, 0xef, 0xf5, 160),
                selection_bg: Color32::from_rgba_unmultiplied(0xad, 0xd6, 0xff, 190),
                keyword: Color32::from_rgb(0xcf, 0x22, 0x2e),
                control_flow: Color32::from_rgb(0xcf, 0x22, 0x2e),
                string: Color32::from_rgb(0x0a, 0x30, 0x69),
                comment: Color32::from_rgb(0x6e, 0x77, 0x81),
                number: Color32::from_rgb(0x05, 0x50, 0xae),
                ty: Color32::from_rgb(0x95, 0x38, 0x00),
                function: Color32::from_rgb(0x82, 0x50, 0xdf),
                macro_call: Color32::from_rgb(0x05, 0x50, 0xae),
                lifetime: Color32::from_rgb(0xbf, 0x39, 0x89),
                attribute: Color32::from_rgb(0x11, 0x63, 0x29),
                operator: Color32::from_rgb(0x24, 0x29, 0x2f),
                normal: Color32::from_rgb(0x24, 0x29, 0x2f),
                line_number: Color32::from_rgb(0x8c, 0x95, 0x9f),
                line_number_active: Color32::from_rgb(0x24, 0x29, 0x2f),
                indent_guide: Color32::from_rgba_unmultiplied(0xd0, 0xd7, 0xde, 160),
                indent_guide_active: Color32::from_rgba_unmultiplied(0x09, 0x69, 0xda, 210),
                error_squiggly: Color32::from_rgb(0xcf, 0x22, 0x2e),
                warning_squiggly: Color32::from_rgb(0x9a, 0x67, 0x00),
                info_squiggly: Color32::from_rgb(0x09, 0x69, 0xda),
            },
        }
    }

    pub fn color_of(&self, kind: TokKind) -> Color32 {
        match kind {
            TokKind::Keyword => self.keyword,
            TokKind::ControlFlow => self.control_flow,
            TokKind::Str => self.string,
            TokKind::Comment => self.comment,
            TokKind::Number => self.number,
            TokKind::Ty => self.ty,
            TokKind::Function => self.function,
            TokKind::Macro => self.macro_call,
            TokKind::Lifetime => self.lifetime,
            TokKind::Attribute => self.attribute,
            TokKind::Operator => self.operator,
            TokKind::Normal => self.normal,
        }
    }
}
