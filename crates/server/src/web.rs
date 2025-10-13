use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post},
};
use axum_extra::routing::TypedPath;

use crate::{db, telemetry::NodeManager};

pub mod assets;
pub mod handler;
pub mod view;

pub struct State {
    pub node_manager: NodeManager,
    pub metrics_store: db::Store,
}

pub type AppState = Arc<State>;

pub fn app(node_manager: NodeManager, metrics_store: db::Store) -> Router {
    let state = Arc::new(State {
        node_manager,
        metrics_store,
    });

    Router::new()
        .route(handler::node::NodeList::PATH, get(handler::node::list))
        .route(handler::node::NodeViewRoute::PATH, get(handler::node::view))
        .route(
            handler::node::NodeEditConfigFormRoute::PATH,
            get(handler::node::edit_config_form),
        )
        .route(
            handler::node::NodeUpdateConfigRoute::PATH,
            post(handler::node::update_config),
        )
        .nest_service(assets::Assets::PATH, assets::Assets::service())
        .with_state(state)
}
