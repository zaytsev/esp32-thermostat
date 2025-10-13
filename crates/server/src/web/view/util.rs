use maud::{Render, html};

use crate::web::assets::Assets;

pub enum Asset<'a> {
    Css(&'a str),
    Js { path: &'a str, module: bool },
}

impl<'a> Asset<'a> {
    pub const fn js(path: &'a str) -> Self {
        Self::Js {
            path,
            module: false,
        }
    }

    pub const fn js_mod(path: &'a str) -> Self {
        Self::Js { path, module: true }
    }

    pub fn href(&self) -> String {
        let file = match self {
            Asset::Css(path) => format!("css/{}", path),
            Asset::Js { path, .. } => format!("js/{}", path),
        };

        let entry = Assets::get(&file).unwrap_or_else(|| panic!("no such asset: {file}"));

        let hash = entry.metadata.sha256_hash();
        let fingerprint = u64::from_le_bytes(hash[..8].try_into().unwrap());
        let suffix = base62::encode(fingerprint);

        format!("/assets/{file}?v={suffix}")
    }
}

impl<'a> Render for Asset<'a> {
    fn render(&self) -> maud::Markup {
        let href = self.href();
        match self {
            Asset::Css(_) => html! {
                link rel="stylesheet" type="text/css" href=(href);
            },
            Asset::Js { module, .. } => html! {
                @if *module {
                    script type="module" src=(href) { }
                } @else {
                    script src=(href) { }
                }
            },
        }
    }
}
