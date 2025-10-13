use axum_embed::ServeEmbed;
use axum_extra::routing::TypedPath;
use rust_embed::Embed;

#[derive(Embed, Clone, TypedPath)]
#[folder = "assets/"]
#[typed_path("/assets")]
pub struct Assets;

impl Assets {
    pub fn service() -> ServeEmbed<Assets> {
        ServeEmbed::<Assets>::new()
    }
}
