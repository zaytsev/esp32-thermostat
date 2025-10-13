use maud::{DOCTYPE, Markup, html};

use crate::web::view::util::Asset;

pub mod model;
pub mod node;
pub mod util;

pub fn base_page(_head_assets: &[Asset], title: &str, content: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html {
            head {
                meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no";
                meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover";                meta charset="UTF-8";
                meta name="apple-mobile-web-app-capable" content="yes";
                meta name="apple-mobile-web-app-status-bar-style" content="black-translucent";

                (Asset::Css("beer.min.css"))
                (Asset::Css("unpoly.min.css"))
                (Asset::Css("main.css"))
                (Asset::Js{path:"unpoly.min.js", module : false})
                (Asset::Js{path:"beer.min.js", module : true})
                (Asset::Js{path:"material-dynamic-colors.min.js", module : true})
                (Asset::js_mod("chart.min.js"))
                (Asset::js_mod("chartjs-adapter-date-fns.bundle.min.js"))
                (Asset::Js{path:"main.js", module : false})
                title {
                    (title)
                }
            }
            body {
                main #main .responsive up-main {
                    (content)
                }
            }
        }
    }
}
