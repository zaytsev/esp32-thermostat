use std::fmt::Display;

use axum_extra::routing::TypedPath;
use chrono::{DateTime, Duration, Utc};
use maud::Render;
use serde::Deserialize;

use crate::web::handler::node::{NodeEditConfigFormRoute, NodeUpdateConfigRoute, NodeViewRoute};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct NodeId(pub u64);

impl Render for NodeId {
    fn render_to(&self, buffer: &mut String) {
        use std::fmt::Write;

        let addr = self.0;
        for i in (0..6).rev() {
            if i != 5 {
                buffer.push(':');
            }
            write!(buffer, "{:02x}", (addr >> (i * 8)) & 0xFF).unwrap();
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NodeListItem {
    pub id: NodeId,
    pub name: Option<String>,
    pub addr: Option<String>,
    pub msgs_rx: Option<u64>,
    pub msgs_tx: Option<u64>,
    pub errors: Option<u64>,
    pub inside_temp: f32,
    pub heater_on: bool,
    pub last_active: Option<DateTime<Utc>>,
}

impl NodeListItem {
    pub fn is_active(&self) -> bool {
        self.addr.is_some()
    }

    pub fn node_view_href(&self) -> String {
        NodeViewRoute { id: self.id.0 }.to_uri().to_string()
    }
}

impl Render for NodeListItem {
    fn render_to(&self, buffer: &mut String) {
        if let Some(name) = self.name.as_deref() {
            name.render_to(buffer);
        } else {
            self.id.render_to(buffer);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NodeDetails {
    pub id: NodeId,
    pub name: Option<String>,
    pub target_temp: Option<f32>,
    pub temp_tolerance: Option<f32>,
    pub last_update: Option<DateTime<Utc>>,
    pub inside_temp: Option<f32>,
    pub heater_on: Option<bool>,
    pub connection: Option<NodeNetworkInfo>,
}

impl NodeDetails {
    pub fn is_connected(&self) -> bool {
        self.connection.is_some()
    }

    pub fn edit_config_page_href(&self) -> impl Display {
        NodeEditConfigFormRoute { id: self.id.0 }.to_string()
    }
}

#[derive(Debug, Clone)]
pub struct NodeNetworkInfo {
    pub addr: String,
    pub rx: u64,
    pub tx: u64,
    pub errors: u64,
    pub duration: Duration,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct NodeEditForm {
    pub name: Option<String>,
    pub target_temp: Option<f32>,
    pub temp_tolerance: Option<f32>,
}

impl NodeEditForm {
    pub fn form_action_href(&self, id: u64) -> impl Display {
        NodeUpdateConfigRoute { id }.to_string()
    }

    pub fn view_href(&self, id: u64) -> impl Display {
        NodeViewRoute { id }.to_string()
    }
}

pub mod chart {

    // src/web/chart.rs

    use chrono::TimeZone;
    use maud::{Markup, PreEscaped, Render, html};
    use serde::Serialize;
    use std::{collections::HashMap, fmt::Write};

    // --- 1. Basic Enums and Helper Structs ---

    /// Represents a color value.
    #[derive(Clone, Debug)]
    pub enum Color {
        Hex(u32),
        Rgba(u8, u8, u8, f32),
        CssProp(String),
    }

    impl Serialize for Color {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            let mut buf = String::with_capacity(32);
            match self {
                Color::Hex(value) => write!(buf, "#{value:06x}").unwrap(),
                Color::Rgba(r, g, b, a) => write!(buf, "rgba({r}, {g}, {b}, {a})").unwrap(),
                Color::CssProp(s) => buf.push_str(s),
            };
            serializer.serialize_str(&buf)
        }
    }

    impl Color {
        pub fn hex(hex: u32) -> Self {
            Color::Hex(hex)
        }

        pub fn rgba(r: u8, g: u8, b: u8, a: f32) -> Self {
            Color::Rgba(r, g, b, a)
        }
    }

    /// The type of chart to render.
    #[derive(Copy, Clone, Debug, Serialize)]
    #[serde(rename_all = "lowercase")]
    pub enum ChartType {
        Line,
        Bar,
        // Add other types as needed
    }

    impl Default for ChartType {
        fn default() -> Self {
            Self::Line
        }
    }

    #[derive(Clone, Debug, Serialize)]
    #[serde(untagged)]
    pub enum DataPoint {
        /// For time-series charts. Serializes to `[timestamp, y]`.
        TimeSeries(i64, f32),
        TimeSeriesXY {
            x: i64,
            y: Option<f32>,
        },
        /// For XY charts. Serializes to `[x, y]`.
        XY(f32, f32),
        /// For categorical charts (using a `labels` array). Serializes to just the `y` value.
        Categorical(f32),
    }

    impl<T: TimeZone> From<(chrono::DateTime<T>, f32)> for DataPoint {
        fn from((ts, val): (chrono::DateTime<T>, f32)) -> Self {
            Self::TimeSeriesXY {
                x: ts.timestamp_millis(),
                y: Some(val),
            }
        }
    }

    #[derive(Serialize, Debug, Default)]
    pub struct ChartConfig {
        #[serde(rename = "type")]
        pub chart_type: ChartType,
        pub data: ChartData,
        pub options: ChartOptions,
    }

    /// The `data` property of the configuration.
    #[derive(Serialize, Debug, Default)]
    pub struct ChartData {
        #[serde(skip_serializing_if = "Option::is_none")]
        pub labels: Option<Vec<String>>,
        pub datasets: Vec<ChartDataset>,
    }

    /// A single dataset configuration.
    #[derive(Serialize, Debug, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct ChartDataset {
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "type")]
        pub chart_type: Option<ChartType>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub label: Option<String>,
        pub data: Vec<DataPoint>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub border_color: Option<Color>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub background_color: Option<Color>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub fill: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub tension: Option<f32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub parsing: Option<bool>,
        #[serde(rename = "xAxisID")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub x_axis_id: Option<AxisId>,
        #[serde(rename = "yAxisID")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub y_axis_id: Option<AxisId>,
        #[serde(rename = "rAxisID")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub r_axis_id: Option<AxisId>,
    }

    /// The `options` property of the configuration.
    #[derive(Serialize, Debug, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct ChartOptions {
        pub animation: bool,
        pub responsive: bool,
        #[serde(rename = "maintainAspectRatio")]
        pub maintain_aspect_ratio: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub scales: Option<Scales>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub plugins: Option<Plugins>,
    }

    #[derive(Serialize, Debug, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct CommonScaleConfig {
        #[serde(skip_serializing_if = "Option::is_none")]
        pub position: Option<Position>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub display: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub title: Option<ScaleTitle>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub grid: Option<GridConfig>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub min: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub max: Option<f64>,
    }

    #[derive(Serialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct TimeScaleConfig {
        #[serde(flatten)]
        pub common: CommonScaleConfig,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub time: Option<TimeConfig>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub bounds: Option<ScaleBounds>,
    }

    /// The main enum for a scale configuration.
    /// `#[serde(tag = "type")]` tells Serde to add a `"type": "variant_name"` field.
    /// `rename_all = "lowercase"` ensures the variant name is serialized in lowercase.
    #[derive(Serialize, Debug)]
    #[serde(tag = "type", rename_all = "lowercase")]
    pub enum Scale {
        Linear(CommonScaleConfig),
        Time(TimeScaleConfig),
        TimeSeries(TimeScaleConfig),
    }

    #[derive(Serialize, Debug, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct TimeConfig {
        #[serde(skip_serializing_if = "Option::is_none")]
        pub tooltip_format: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub unit: Option<TimeUnit>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub display_formats: Option<DisplayFormats>,
    }

    #[derive(Serialize, Debug, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct DisplayFormats {
        #[serde(skip_serializing_if = "Option::is_none")]
        pub quarter: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub hour: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub day: Option<String>,
    }

    #[derive(Serialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct ScaleTitle {
        pub display: bool,
        pub text: String,
    }

    #[derive(Serialize, Debug, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct GridConfig {
        pub display: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub color: Option<Color>,
    }

    #[derive(Serialize, Debug, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct TicksConfig {
        #[serde(skip_serializing_if = "Option::is_none")]
        pub source: Option<TicksSource>,
    }

    #[derive(Copy, Clone, Debug, Serialize)]
    #[serde(rename_all = "lowercase")]
    pub enum ScaleType {
        Time,
        Linear,
        Logarithmic,
        Category,
    }

    #[derive(Copy, Clone, Debug, Serialize)]
    #[serde(rename_all = "lowercase")]
    pub enum TimeUnit {
        Millisecond,
        Second,
        Minute,
        Hour,
        Day,
        Week,
        Month,
        Quarter,
        Year,
    }

    #[derive(Copy, Clone, Debug, Serialize)]
    #[serde(rename_all = "lowercase")]
    pub enum Position {
        Top,
        Bottom,
        Left,
        Right,
    }

    #[derive(Copy, Clone, Debug, Serialize)]
    #[serde(rename_all = "lowercase")]
    pub enum TicksSource {
        Auto,
        Data,
        Labels,
    }

    #[derive(Copy, Clone, Debug, Serialize)]
    #[serde(rename_all = "lowercase")]
    pub enum ScaleBounds {
        Data,
        Ticks,
    }

    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum AxisId {
        X,
        Y,
        R,
        Custom(String),
    }

    impl Serialize for AxisId {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            match self {
                AxisId::X => serializer.serialize_str("x"),
                AxisId::Y => serializer.serialize_str("y"),
                AxisId::R => serializer.serialize_str("r"),
                AxisId::Custom(val) => serializer.serialize_str(&val),
            }
        }
    }

    // --- The Scales struct remains the same ---
    #[derive(Serialize, Debug, Default)]
    pub struct Scales {
        #[serde(flatten)]
        #[serde(skip_serializing_if = "HashMap::is_empty")]
        pub axes: HashMap<AxisId, Scale>,
    }

    /// Configuration for plugins like title, legend, tooltip.
    #[derive(Serialize, Debug, Default)]
    pub struct Plugins {
        #[serde(skip_serializing_if = "Option::is_none")]
        pub title: Option<Title>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub legend: Option<Legend>,
    }

    #[derive(Serialize, Debug)]
    pub struct Title {
        pub display: bool,
        pub text: String,
    }

    #[derive(Serialize, Debug)]
    pub struct Legend {
        pub display: bool,
        pub position: Position,
    }

    pub struct ChartConfigBuilder {
        chart_type: ChartType,
        datasets: Vec<ChartDataset>,
        options: ChartOptions,
    }

    impl ChartConfigBuilder {
        pub fn new(chart_type: ChartType) -> Self {
            Self {
                chart_type,
                datasets: Vec::new(),
                options: ChartOptions {
                    responsive: true,
                    maintain_aspect_ratio: false,
                    ..Default::default()
                },
            }
        }

        pub fn x_axis(mut self, config: Scale) -> Self {
            let scales = self.options.scales.get_or_insert(Scales::default());
            scales.axes.insert(AxisId::X, config);
            self
        }

        pub fn y_axis(mut self, config: Scale) -> Self {
            let scales = self.options.scales.get_or_insert(Scales::default());
            scales.axes.insert(AxisId::Y, config);
            self
        }

        pub fn r_axis(mut self, config: Scale) -> Self {
            let scales = self.options.scales.get_or_insert(Scales::default());
            scales.axes.insert(AxisId::R, config);
            self
        }

        pub fn axis(mut self, id: AxisId, config: Scale) -> Self {
            let scales = self.options.scales.get_or_insert(Scales::default());
            scales.axes.insert(id, config);
            self
        }

        pub fn add_dataset(mut self, dataset: ChartDataset) -> Self {
            self.datasets.push(dataset);
            self
        }

        pub fn options(mut self, options: ChartOptions) -> Self {
            self.options = options;
            self
        }

        pub fn build(self) -> ChartConfig {
            ChartConfig {
                chart_type: self.chart_type,
                data: ChartData {
                    labels: None, // Can be added later if needed
                    datasets: self.datasets,
                },
                options: self.options,
            }
        }
    }

    // --- 4. Final Maud Component ---

    /// The final renderable component that creates the `<canvas>` element.
    #[derive(Debug, Default)]
    pub struct Chart {
        pub id: Option<String>,
        pub config: ChartConfig,
        pub width: Option<u32>,
        pub height: Option<u32>,
        pub style: Option<String>, // For custom CSS like "height: 400px;"
    }

    impl Render for Chart {
        fn render(&self) -> Markup {
            let data_json =
                serde_json::to_string(&self.config).expect("ChartConfig should be serializable");

            // let data_attr = format!(r#"up-data='{}'"#, data_json);
            html! {
                canvas.chart
                    id=[self.id.as_deref()]
                    width=[self.width]
                    height=[self.height]
                    style=[self.style.as_deref()]
                    up-data=(data_json) {}
            }
        }
    }
}
