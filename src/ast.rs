// NovaRS — AST Outline Parser & Symbol Navigation Tree

use eframe::egui::Color32;

#[derive(Clone, Debug)]
pub struct AstItem {
    pub kind: &'static str,
    pub name: String,
    pub line: usize,
    pub depth: usize,
}

pub fn collect_ast_items(items: &[syn::Item], depth: usize, out: &mut Vec<AstItem>) {
    for item in items {
        match item {
            syn::Item::Mod(m) => {
                out.push(AstItem {
                    kind: "mod",
                    name: m.ident.to_string(),
                    line: m.ident.span().start().line,
                    depth,
                });
                if let Some((_, inner)) = &m.content {
                    collect_ast_items(inner, depth + 1, out);
                }
            }
            syn::Item::Struct(s) => out.push(AstItem {
                kind: "struct",
                name: s.ident.to_string(),
                line: s.ident.span().start().line,
                depth,
            }),
            syn::Item::Enum(e) => out.push(AstItem {
                kind: "enum",
                name: e.ident.to_string(),
                line: e.ident.span().start().line,
                depth,
            }),
            syn::Item::Trait(t) => {
                out.push(AstItem {
                    kind: "trait",
                    name: t.ident.to_string(),
                    line: t.ident.span().start().line,
                    depth,
                });
                for ti in &t.items {
                    if let syn::TraitItem::Fn(f) = ti {
                        out.push(AstItem {
                            kind: "fn",
                            name: f.sig.ident.to_string(),
                            line: f.sig.ident.span().start().line,
                            depth: depth + 1,
                        });
                    }
                }
            }
            syn::Item::Fn(f) => out.push(AstItem {
                kind: "fn",
                name: f.sig.ident.to_string(),
                line: f.sig.ident.span().start().line,
                depth,
            }),
            syn::Item::Const(c) => out.push(AstItem {
                kind: "const",
                name: c.ident.to_string(),
                line: c.ident.span().start().line,
                depth,
            }),
            syn::Item::Type(ty) => out.push(AstItem {
                kind: "type",
                name: ty.ident.to_string(),
                line: ty.ident.span().start().line,
                depth,
            }),
            syn::Item::Impl(imp) => {
                for impl_item in &imp.items {
                    if let syn::ImplItem::Fn(f) = impl_item {
                        out.push(AstItem {
                            kind: "fn",
                            name: f.sig.ident.to_string(),
                            line: f.sig.ident.span().start().line,
                            depth: depth + 1,
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

pub fn ast_kind_color(kind: &str, dark: bool) -> Color32 {
    if dark {
        match kind {
            "mod" => Color32::from_rgb(0xe5, 0xc0, 0x7b),
            "struct" => Color32::from_rgb(0x61, 0xaf, 0xef),
            "enum" => Color32::from_rgb(0xc6, 0x78, 0xdd),
            "trait" => Color32::from_rgb(0x56, 0xb6, 0xc2),
            "const" | "type" => Color32::from_rgb(0xd1, 0x9a, 0x66),
            _ => Color32::from_rgb(0x98, 0xc3, 0x79),
        }
    } else {
        match kind {
            "mod" => Color32::from_rgb(0xb7, 0x6b, 0x01),
            "struct" => Color32::from_rgb(0x40, 0x78, 0xbf),
            "enum" => Color32::from_rgb(0x98, 0x68, 0x01),
            "trait" => Color32::from_rgb(0x0e, 0x8a, 0x84),
            "const" | "type" => Color32::from_rgb(0x95, 0x38, 0x00),
            _ => Color32::from_rgb(0x50, 0xa1, 0x4f),
        }
    }
}
