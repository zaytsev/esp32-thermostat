use chrono::{DateTime, Utc};
use maud::{Markup, html};

use crate::{
    db::{DataPoint, Granularity},
    web::view::{
        base_page,
        model::{
            NodeDetails, NodeEditForm, NodeListItem,
            chart::{
                self, AxisId, Chart, ChartConfigBuilder, ChartDataset, ChartType, Color,
                CommonScaleConfig, DisplayFormats, GridConfig, Scale, ScaleBounds, ScaleTitle,
                TimeConfig, TimeScaleConfig, TimeUnit,
            },
        },
        util::Asset,
    },
};

pub fn list(nodes: &[NodeListItem]) -> Markup {
    let content = if nodes.is_empty() {
        html! {
            article .medium.middle-align.center-align {
                div {
                    i .extra { "hub" }
                    h5 { "No registered nodes found" }
                }
            }
        }
    } else {
        html! {
            article {
                ul.list.border {
                    @for node in nodes {
                        li {
                            i .extra.green-text[node.is_active()] {
                                "wifi"
                            }
                            i .extra.red-text[node.heater_on] {
                                @if node.heater_on {
                                    "mode_heat"
                                } @else {
                                    "mode_heat_off"
                                }
                            }
                            div .max {
                                a .max href=(node.node_view_href()) up-follow up-transition="move-left" {
                                    h5 {
                                        (node)
                                    }
                                }
                            }
                            i .extra {
                                "chevron_right"
                            }
                        }
                    }
                }
            }
        }
    };
    base_page(&[], "Index", content)
}

pub fn view(
    node: NodeDetails,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    step: Granularity,
    data_points: Vec<DataPoint>,
) -> Markup {
    let start = from.timestamp_millis();
    let end = to.timestamp_millis();
    let step = step.duration().num_milliseconds();
    let total_data_points = ((end - start) / step) as usize + 1;

    let mut min_temp_data = Vec::with_capacity(total_data_points);
    let mut max_temp_data = Vec::with_capacity(total_data_points);
    let mut avg_temp_data = Vec::with_capacity(total_data_points);
    let mut heater_data = Vec::with_capacity(total_data_points);
    let mut data_points_iter = data_points.iter().peekable();
    let mut min_temp_value = f32::INFINITY;
    let mut max_temp_value = f32::NEG_INFINITY;

    for ts in (start..end).step_by(step as usize) {
        let (min_temp, max_temp, avg_temp, heater_ratio) = if let Some(pt) =
            data_points_iter.next_if(|pt| pt.ts.timestamp_millis() <= ts + step)
        {
            min_temp_value = min_temp_value.min(pt.min_temp);
            max_temp_value = max_temp_value.max(pt.max_temp);
            (
                Some(pt.min_temp),
                Some(pt.max_temp),
                Some(pt.avg_temp),
                Some(pt.heater_ratio * 100.0),
            )
        } else {
            (None, None, None, None)
        };

        min_temp_data.push(chart::DataPoint::TimeSeriesXY { x: ts, y: min_temp });
        max_temp_data.push(chart::DataPoint::TimeSeriesXY { x: ts, y: max_temp });
        avg_temp_data.push(chart::DataPoint::TimeSeriesXY { x: ts, y: avg_temp });
        heater_data.push(chart::DataPoint::TimeSeriesXY {
            x: ts,
            y: heater_ratio,
        });
    }

    let x_scale = Scale::TimeSeries(TimeScaleConfig {
        common: CommonScaleConfig {
            min: Some(start as f64),
            max: Some(end as f64),
            title: Some(ScaleTitle {
                display: true,
                text: "Time".to_string(),
            }),
            grid: Some(GridConfig {
                display: false,
                color: None, // color: Some(Color::hex(0xe5e7eb)),
            }),
            ..Default::default()
        },
        time: Some(TimeConfig {
            tooltip_format: Some("dd-MM-yyyy HH:mm".to_string()),
            unit: Some(TimeUnit::Hour),
            display_formats: Some(DisplayFormats {
                hour: Some("HH:mm".to_string()),
                day: Some("dd MMM".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }),
        bounds: Some(ScaleBounds::Data),
    });

    let min_temp = (min_temp_value as f64 - 1.0).floor();
    let max_temp = (max_temp_value as f64 + 1.0).ceil();

    let y_scale = Scale::Linear(CommonScaleConfig {
        min: Some(min_temp),
        max: Some(max_temp),
        title: Some(ScaleTitle {
            display: true,
            text: "Temperature (°C)".to_string(),
        }),
        grid: Some(GridConfig {
            display: true,
            color: Some(Color::hex(0xe5e7eb)),
        }),
        ..Default::default() // And from here.
    });

    let heater_scale = Scale::Linear(CommonScaleConfig {
        min: Some(0.0f64),
        max: Some(100.0f64),
        position: Some(chart::Position::Right),
        title: Some(ScaleTitle {
            display: true,
            text: "Usage %".to_string(),
        }),
        grid: Some(GridConfig {
            display: false,
            color: None,
        }),
        ..Default::default()
    });

    let heater_axis_id = AxisId::Custom("h".to_string());

    let chart_config = ChartConfigBuilder::new(ChartType::Line)
        .add_dataset(ChartDataset {
            chart_type: Some(ChartType::Line),
            label: Some("Avg Temp (°C)".to_string()),
            data: avg_temp_data,
            parsing: Some(false),
            ..Default::default()
        })
        .add_dataset(ChartDataset {
            chart_type: Some(ChartType::Line),
            label: Some("Min Temp (°C)".to_string()),
            data: min_temp_data,
            parsing: Some(false),
            ..Default::default()
        })
        .add_dataset(ChartDataset {
            chart_type: Some(ChartType::Line),
            label: Some("Max Temp (°C)".to_string()),
            data: max_temp_data,
            parsing: Some(false),
            ..Default::default()
        })
        .add_dataset(ChartDataset {
            chart_type: Some(ChartType::Bar),
            label: Some("Heater Usage (%)".to_string()),
            data: heater_data,
            parsing: Some(false),
            y_axis_id: Some(heater_axis_id.clone()),
            ..Default::default()
        })
        .x_axis(x_scale)
        .y_axis(y_scale)
        .axis(heater_axis_id, heater_scale)
        .build();

    let chart = Chart {
        config: chart_config,
        height: Some(450),
        ..Default::default()
    };

    base_page(
        &[],
        "View",
        html! {
            article {
                header {
                    div .row {
                            @if node.name.is_some() {
                                h5 .max {
                                    (node.name.as_deref().unwrap())
                                }
                                h6 .m.l {
                                    (node.id)
                                }
                            } @else {
                                h5 .max {
                                    (node.id)
                                }
                            }
                        a .button.border href=(node.edit_config_page_href()) up-follow {
                            i { "edit" }
                            span .m.l { "Edit" }
                        }
                    }
                }
                div .grid.no-space {
                        div .s12.m6.l6.vertical-margin {
                            div .row {
                                i { "history" }
                                div .m.l {
                                    "Last update:"
                                }
                                @if let Some(timestamp) = node.last_update.as_ref() {
                                    time .max datetime=(timestamp.to_rfc3339());
                                } @else {
                                   "N/A"
                                }
                            }
                        }
                        div .s12.m6.l6.vertical-margin {
                            div .row {
                                i .medium { "thermometer" }
                                div .m.l {
                                    "Temperature"
                                }
                                @if let Some(inside_temp) = node.inside_temp {
                                    div {
                                         (inside_temp) "°C"
                                    }
                                } @else {
                                    "N/A"
                                }
                            }
                        }
                        div .s12.m6.l6.vertical-align {
                            div .row {
                                i .medium {
                                    "mode_heat"
                                }
                                div .m.l {
                                    "Heater"
                                }
                                @if let Some(heater_on) = node.heater_on {
                                    @if heater_on {
                                        div .primary { "ON" }
                                    } @else {
                                        div .secondary { "OFF" }
                                    }
                                } @else {
                                    div { "N/A" }
                                }
                            }
                        }
                        div .s12.m6.l6.vertical-align {
                            div .row {
                                i .medium {
                                    "device_thermostat"
                                }
                                div .m.l {
                                    "Thermostat"
                                }
                                div .row.max{
                                    @if let Some(target_temp) = node.target_temp {
                                        p { "Target" }
                                        strong { (target_temp) }
                                        @if let Some(tolerance) = node.temp_tolerance {
                                        span { "±" }
                                        strong { (tolerance) }
                                        }
                                        p { "°C" }
                                    } @else {
                                        "N/A"
                                    }
                                }
                            }
                        }
                    @if node.is_connected() {
                        @let net = node.connection.as_ref().unwrap();
                        div .s12.m6.l6.vertical-margin {
                            div .row.left-align.middle-align {
                                i .medium { "lan" }
                                div .m.l { "Address"}
                                div .max {
                                    (net.addr)
                                }
                            }
                        }
                        div .s4.m2.l2.vertical-margin {
                            div .row.middle-align {
                                i .medium { "arrow_downward" }
                                p .m.l { "Received"}
                                p .max { (net.rx) }
                            }
                        }
                        div .s4.m2.l2.vertical-margin {
                            div .row.middle-align {
                                i .medium { "arrow_upward" }
                                p .m.l { "Sent"}
                                p .max { (net.tx) }
                            }
                        }
                        div .s4.m2.l2.vertical-margin {
                            div .row.middle-align {
                                @let has_errors = net.errors > 0;
                                i .medium.error-text[has_errors] { "error" }
                                p .m.l { "Errors"}
                                p .max.error-text[has_errors] { (net.errors) }
                            }
                        }
                    }
                    div .s12.m12.l12 {
                        (chart)
                    }
                }
                footer {
                    nav .center-align {
                        a .button.border href=(node.edit_config_page_href()) up-follow {
                            i { "edit" }
                            span { "Edit" }
                        }
                    }
                }
            }
        },
    )
}

pub fn edit_form(node_id: u64, form: NodeEditForm) -> Markup {
    base_page(
        &[],
        "Edit",
        html! {
            form method="post" action=(form.form_action_href(node_id)) up-submit up-history="true" up-location=(form.view_href(node_id)) {
                article {
                        div .field.border {
                            input name="name" type="text" value=[form.name.as_ref()];
                            span .helper { "Node name" }
                        }
                        div {
                            div .field.middle-align {
                                label .slider.medium.no-margin {
                                    input name="target_temp" type="range" min="0" max="30" value=[form.target_temp];
                                    span {
                                        i { "device_thermostat" }
                                    }
                                    div .tooltip;

                                }
                                span .helper { "Thermostat target temperature" }
                            }
                        }
                        div {
                            div .field.middle-align {
                                label .slider.medium.no-margin {
                                    input type="range" name="temp_tolerance" min="0.1" max="2" step="0.1" value=[form.temp_tolerance];
                                    span {
                                        i { "thermostat" }
                                    }
                                    div .tooltip;

                                }
                                span .helper { "Thermostat temperature tolerance" }
                            }
                        }
                        footer {
                            nav .medium.right-align {
                                a .button.border.round href=(form.view_href(node_id)) up-follow {
                                    i { "arrow_back" }
                                    "Cancel"
                                }
                                button type="submit" value = "save" {
                                    i { "check" }
                                    "Save"
                                }
                            }
                        }
                }
            }
        },
    )
}
