use std::collections::HashMap;

use axum::{
    Form,
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect},
};
use axum_extra::routing::TypedPath;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    db::{Granularity, NodeConfig, NodeDetails},
    telemetry::NodeInfo,
    web::{
        AppState,
        view::{
            self,
            model::{NodeEditForm, NodeId, NodeListItem},
        },
    },
};

#[derive(TypedPath)]
#[typed_path("/")]
pub struct NodeList;

pub async fn list(state: State<AppState>) -> impl IntoResponse {
    let active_nodes = state.node_manager.list_nodes().await;
    let to = Utc::now();
    let from = to - Duration::days(1);
    let metrics = state
        .metrics_store
        .all_nodes_latest_record_in_range(from, to)
        .await
        .unwrap();

    let active_nodes_by_id: HashMap<u64, NodeInfo> =
        active_nodes.into_iter().map(|n| (n.node_id, n)).collect();

    let items = metrics
        .into_iter()
        .map(|m| {
            if let Some(NodeInfo {
                addr,
                connected,
                last_active,
                msgs_rx,
                msgs_tx,
                errors,
                connected_at,
                ..
            }) = active_nodes_by_id.get(&m.node_id)
            {
                NodeListItem {
                    id: NodeId(m.node_id),
                    name: None,
                    addr: Some(addr.to_string()),
                    msgs_rx: Some(*msgs_rx),
                    msgs_tx: Some(*msgs_tx),
                    errors: Some(*errors),
                    inside_temp: m.inside_temp,
                    heater_on: m.heater_on,
                    last_active: Some(*connected_at + (*last_active - *connected)),
                }
            } else {
                NodeListItem {
                    id: NodeId(m.node_id),
                    name: None,
                    addr: None,
                    msgs_rx: None,
                    msgs_tx: None,
                    errors: None,
                    inside_temp: m.inside_temp,
                    heater_on: m.heater_on,
                    last_active: Some(m.received_at),
                }
            }
        })
        .collect::<Vec<NodeListItem>>();

    view::node::list(&items)
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/node/{id}")]
pub struct NodeViewRoute {
    pub id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeViewParams {
    pub step: Option<Granularity>,
    #[serde(rename = "from")]
    pub start_date: Option<DateTime<Utc>>,
    #[serde(rename = "to")]
    pub end_date: Option<DateTime<Utc>>,
}

pub async fn view(
    Path(node_id): Path<u64>,
    Query(NodeViewParams {
        step,
        start_date,
        end_date,
    }): Query<NodeViewParams>,
    state: State<AppState>,
) -> impl IntoResponse {
    let mut node = view::model::NodeDetails {
        id: NodeId(node_id),
        ..Default::default()
    };

    if let Some(NodeDetails {
        name,
        target_temp,
        temp_tolerance,
        last_update,
        inside_temp,
        heater_on,
        ..
    }) = state
        .metrics_store
        .find_node_details(node_id)
        .await
        .unwrap()
    {
        node.name = name;
        node.target_temp = target_temp;
        node.temp_tolerance = temp_tolerance;
        node.last_update = last_update;
        node.inside_temp = inside_temp;
        node.heater_on = heater_on;
    }

    if let Some(NodeInfo {
        addr,
        connected,
        last_active,
        msgs_rx,
        msgs_tx,
        errors,
        ..
    }) = state.node_manager.find_node_by_id(node_id).await
    {
        node.connection = Some(view::model::NodeNetworkInfo {
            addr: addr.to_string(),
            rx: msgs_rx,
            tx: msgs_tx,
            errors,
            duration: Duration::from_std(last_active - connected).unwrap_or_default(),
        });
    }

    let step = step.unwrap_or(Granularity::Hour);
    let end_date = end_date.unwrap_or_else(|| Utc::now());
    let to = step.ceil(end_date);
    let from = step.floor(start_date.unwrap_or_else(|| {
        end_date
            - step
                .next()
                .map(|g| g.duration())
                .unwrap_or_else(|| Duration::days(365))
    }));
    let data = state
        .metrics_store
        .range(node_id, step, from, to)
        .await
        .unwrap();

    view::node::view(node, from, to, step, data)
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/node/{id}/edit")]
pub struct NodeEditConfigFormRoute {
    pub id: u64,
}

pub async fn edit_config_form(
    Path(node_id): Path<u64>,
    state: State<AppState>,
) -> impl IntoResponse {
    let form_fields = if let Some(NodeConfig {
        name,
        target_temp,
        temp_tolerance,
    }) = state.metrics_store.find_node_config(node_id).await.unwrap()
    {
        NodeEditForm {
            name,
            target_temp,
            temp_tolerance,
        }
    } else {
        NodeEditForm::default()
    };
    view::node::edit_form(node_id, form_fields)
}

#[derive(TypedPath, Deserialize)]
#[typed_path("/node/{id}")]
pub struct NodeUpdateConfigRoute {
    pub id: u64,
}

pub async fn update_config(
    Path(node_id): Path<u64>,
    state: State<AppState>,
    Form(fields): Form<NodeEditForm>,
) -> impl IntoResponse {
    let cfg = NodeConfig {
        name: fields.name,
        target_temp: fields.target_temp,
        temp_tolerance: fields.temp_tolerance,
    };

    state
        .metrics_store
        .save_node_config(node_id, cfg)
        .await
        .unwrap();

    if let (Some(temp), Some(tolerance)) = (fields.target_temp, fields.temp_tolerance) {
        state
            .node_manager
            .set_target_temp(node_id, temp, tolerance)
            .await;
    }

    let location = NodeViewRoute { id: node_id }.to_string();
    Redirect::to(&location).into_response()
}
