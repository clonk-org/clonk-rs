use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::ClientId;

pub const DEFAULT_GRAPH_BACKLOG: usize = 256;
pub const PLAYER_GRAPH_BACKLOG: usize = DEFAULT_GRAPH_BACKLOG * 20;
pub const CONTROL_GRAPH_AVERAGE: i32 = 100;
pub const FORWARD_AVERAGE_FACTOR: i32 = 4;

const DEFAULT_GRAPH_COLOR: u32 = 0x7fff_0000;
const NET_INPUT_COLOR: u32 = 0x0000_ff00;
const NET_OUTPUT_COLOR: u32 = 0x00ff_0000;
const CLIENT_GRAPH_COLORS: [u32; 11] = [
    0x00ff_0000,
    0x0000_ff00,
    0x00ff_ff00,
    0x007f_7fff,
    0x00ff_ffff,
    0x0000_ffff,
    0x00ff_00ff,
    0x007f_7f7f,
    0x00ff_7f7f,
    0x007f_ff7f,
    0x0000_00ff,
];

fn readable_player_graph_color(color: u32) -> u32 {
    let color = color & 0x00ff_ffff;
    let red = (color >> 16) & 0xff;
    let green = (color >> 8) & 0xff;
    let blue = color & 0xff;
    let lightness = red * 50 + green * 87 + blue * 27;
    if lightness >= 16_575 {
        return color;
    }
    let increment = (16_575 - lightness) / 164;
    ((red + increment).min(255) << 16)
        | ((green + increment).min(255) << 8)
        | (blue + increment).min(255)
}

/// A fixed-size, integer-time graph matching `C4TableGraph`'s ring and
/// asymmetric weighted averaging.
#[derive(Debug)]
pub struct TableGraph {
    title: String,
    color: u32,
    values: Vec<f32>,
    averaged_values: Option<Vec<f32>>,
    backlog_pos: usize,
    wrapped: bool,
    initial_start_time: i32,
    time: i32,
    averaged_time: i32,
    average_range: i32,
    multiplier: f32,
    dump_file: Option<PathBuf>,
}

impl Default for TableGraph {
    fn default() -> Self {
        Self::new(DEFAULT_GRAPH_BACKLOG, 0)
    }
}

impl TableGraph {
    pub fn new(backlog_length: usize, start_time: i32) -> Self {
        assert!(backlog_length > 0, "graph backlog must not be empty");
        assert!(
            i32::try_from(backlog_length).is_ok(),
            "graph backlog must fit the C++ time type"
        );
        Self {
            title: "Network graph".to_string(),
            color: DEFAULT_GRAPH_COLOR,
            values: vec![0.0; backlog_length],
            averaged_values: None,
            backlog_pos: 0,
            wrapped: false,
            initial_start_time: start_time,
            time: start_time,
            averaged_time: start_time,
            average_range: 1,
            multiplier: 1.0,
            dump_file: None,
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = title.into();
    }

    pub fn color(&self) -> u32 {
        self.color
    }

    pub fn set_color(&mut self, color: u32) {
        self.color = color;
    }

    pub fn backlog_length(&self) -> usize {
        self.values.len()
    }

    pub fn average_time(&self) -> i32 {
        self.average_range
    }

    pub fn multiplier(&self) -> f32 {
        self.multiplier
    }

    pub fn set_multiplier(&mut self, multiplier: f32) {
        self.multiplier = multiplier;
    }

    pub fn is_empty(&self) -> bool {
        !self.wrapped && self.backlog_pos == 0
    }

    pub fn start_time(&self) -> i32 {
        if self.wrapped {
            self.time - self.values.len() as i32
        } else {
            self.time - self.backlog_pos as i32
        }
    }

    pub fn end_time(&self) -> i32 {
        self.time
    }

    pub fn contains_time(&self, time: i32) -> bool {
        time >= self.start_time() && time < self.end_time()
    }

    /// Records one raw sample. As in C++, callers must invoke [`Self::update`]
    /// before reading an averaged graph.
    pub fn record_value(&mut self, value: f32) {
        self.values[self.backlog_pos] = value;
        self.time += 1;
        self.backlog_pos += 1;
        if self.backlog_pos == self.values.len() {
            self.flush_configured_dump(self.wrapped);
            self.wrapped = true;
            self.backlog_pos = 0;
        }
    }

    pub fn reset(&mut self, start_time: i32) {
        self.flush_configured_dump(self.wrapped);
        self.values.fill(0.0);
        if let Some(averaged) = self.averaged_values.as_mut() {
            averaged.fill(0.0);
        }
        self.backlog_pos = 0;
        self.wrapped = false;
        self.initial_start_time = start_time;
        self.time = start_time;
        self.averaged_time = start_time;
    }

    pub fn set_average_time(&mut self, average_range: i32) {
        assert!(average_range > 0, "graph average time must be positive");
        if self.average_range == average_range {
            return;
        }
        self.average_range = average_range;
        self.averaged_time = self.initial_start_time;
        if average_range == 1 {
            self.averaged_values = None;
            self.averaged_time = self.time;
        }
    }

    /// Refreshes the lazy averaged buffer through the current end time.
    pub fn update(&mut self) {
        if self.average_range == 1 {
            self.averaged_time = self.time;
            return;
        }
        if self.averaged_values.is_none() {
            self.averaged_values = Some(vec![0.0; self.values.len()]);
        }
        if self.averaged_time == self.time {
            return;
        }

        let start_time = self.start_time();
        let forward_range = self.average_range / FORWARD_AVERAGE_FACTOR;
        let first_update = self
            .averaged_time
            .saturating_sub(forward_range)
            .saturating_sub(1)
            .max(start_time);

        for update_time in first_update..self.time {
            let sum_start = update_time
                .saturating_sub(self.average_range)
                .max(start_time);
            let sum_end = update_time
                .saturating_add(forward_range)
                .saturating_add(1)
                .min(self.time);
            let mut sum = 0.0_f32;
            let mut sum_weight = 0.0_f32;
            for sample_time in sum_start..sum_end {
                let distance = (update_time - sample_time).abs();
                let weight = self.average_range as f32 - distance as f32 + 1.0;
                let weighted = self.raw_value(sample_time) * weight;
                sum += weighted;
                sum_weight += weight;
            }
            let index = self.index_for(update_time);
            self.averaged_values
                .as_mut()
                .expect("averaged storage was initialized")[index] = sum / sum_weight;
        }
        self.averaged_time = self.time;
    }

    /// Returns the current averaged and multiplied value. This does not
    /// implicitly call [`Self::update`], matching the C++ graph contract.
    pub fn value(&self, time: i32) -> f32 {
        assert!(
            self.contains_time(time),
            "graph time is outside the backlog"
        );
        let index = self.index_for(time);
        self.averaged_values
            .as_ref()
            .map_or(self.values[index], |averaged| averaged[index])
            * self.multiplier
    }

    pub fn raw_value(&self, time: i32) -> f32 {
        assert!(
            self.contains_time(time),
            "graph time is outside the backlog"
        );
        self.values[self.index_for(time)]
    }

    /// Matches the oracle's misnamed `GetMedianValue`: this is an arithmetic
    /// mean over the half-open interval, after averaging and multiplication.
    pub fn mean_value(&self, start_time: i32, end_time: i32) -> f32 {
        assert!(start_time < end_time, "graph mean range must not be empty");
        if self.is_empty() {
            return 0.0;
        }
        assert!(
            self.contains_time(start_time) && self.contains_time(end_time - 1),
            "graph mean range is outside the backlog"
        );
        let mut sum = 0.0_f32;
        let mut count = 0.0_f32;
        for time in start_time..end_time {
            sum += self.value(time);
            count += 1.0;
        }
        sum / count
    }

    pub fn min_value(&self) -> f32 {
        if self.is_empty() {
            return 0.0;
        }
        let mut minimum = f32::INFINITY;
        for time in self.start_time()..self.end_time() {
            let index = self.index_for(time);
            let value = self
                .averaged_values
                .as_ref()
                .map_or(self.values[index], |averaged| averaged[index]);
            minimum = minimum.min(value);
        }
        minimum * self.multiplier
    }

    pub fn max_value(&self) -> f32 {
        if self.is_empty() {
            return 0.0;
        }
        let mut maximum = f32::NEG_INFINITY;
        for time in self.start_time()..self.end_time() {
            let index = self.index_for(time);
            let value = self
                .averaged_values
                .as_ref()
                .map_or(self.values[index], |averaged| averaged[index]);
            maximum = maximum.max(value);
        }
        maximum * self.multiplier
    }

    /// Writes the currently materialized values without refreshing averaging.
    pub fn write_tsv(&self, writer: &mut impl Write, include_header: bool) -> io::Result<bool> {
        if self.is_empty() {
            return Ok(false);
        }
        if include_header {
            writer.write_all(b"t\tv\r\n")?;
        }
        for time in self.start_time()..self.end_time() {
            write!(writer, "{time}\t{}\r\n", self.value(time))?;
        }
        Ok(true)
    }

    pub fn dump_to_file(&self, path: impl AsRef<Path>, append: bool) -> io::Result<bool> {
        if self.is_empty() {
            return Ok(false);
        }
        let path = path.as_ref();
        let (mut file, include_header) = if append {
            match OpenOptions::new().append(true).open(path) {
                Ok(file) => (file, false),
                Err(_) => (File::create(path)?, true),
            }
        } else {
            (File::create(path)?, true)
        };
        self.write_tsv(&mut file, include_header)
    }

    pub fn set_dump_file(&mut self, path: impl Into<PathBuf>) {
        self.dump_file = Some(path.into());
    }

    pub fn clear_dump_file(&mut self) {
        self.dump_file = None;
    }

    pub fn dump_file(&self) -> Option<&Path> {
        self.dump_file.as_deref()
    }

    fn index_for(&self, time: i32) -> usize {
        usize::try_from(time - self.initial_start_time)
            .expect("valid graph time cannot precede its initial start")
            % self.values.len()
    }

    fn flush_configured_dump(&self, append: bool) {
        if let Some(path) = self.dump_file.as_deref() {
            let _ = self.dump_to_file(path, append);
        }
    }
}

impl Drop for TableGraph {
    fn drop(&mut self) {
        self.flush_configured_dump(self.wrapped);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolRateSample {
    pub tcp: i32,
    pub udp: i32,
}

impl ProtocolRateSample {
    pub const fn new(tcp: i32, udp: i32) -> Self {
        Self { tcp, udp }
    }

    fn total(self) -> i32 {
        self.tcp + self.udp
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientPingSample {
    pub client_id: ClientId,
    /// `None` represents a client without a message connection and records
    /// zero. `Some(-1)` preserves an unmeasured live connection.
    pub lag_ms: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerControlSample {
    pub player_id: i32,
    pub controls: i32,
    pub actions: i32,
}

#[derive(Debug)]
struct PlayerGraphs {
    controls: TableGraph,
    actions: TableGraph,
}

/// A flattened view of one graph or a C++-style graph collection.
#[derive(Debug)]
pub struct NetworkStatsGraph<'a> {
    title: &'static str,
    series: Vec<&'a TableGraph>,
}

impl<'a> NetworkStatsGraph<'a> {
    pub fn title(&self) -> &str {
        self.title
    }

    pub fn series_count(&self) -> usize {
        self.series.len()
    }

    pub fn series(&self, index: usize) -> Option<&'a TableGraph> {
        self.series.get(index).copied()
    }

    pub fn start_time(&self) -> i32 {
        self.series
            .iter()
            .map(|graph| graph.start_time())
            .min()
            .unwrap_or(0)
    }

    pub fn end_time(&self) -> i32 {
        self.series
            .iter()
            .map(|graph| graph.end_time())
            .max()
            .unwrap_or(0)
    }

    pub fn min_value(&self) -> f32 {
        self.series
            .iter()
            .map(|graph| graph.min_value())
            .reduce(f32::min)
            .unwrap_or(0.0)
    }

    pub fn max_value(&self) -> f32 {
        self.series
            .iter()
            .map(|graph| graph.max_value())
            .reduce(f32::max)
            .unwrap_or(0.0)
    }
}

/// Input-driven counterpart of `C4Network2Stats`. Runtime owners provide the
/// frame, rate, ping, and player-count samples without coupling this crate to
/// the application or engine loops.
#[derive(Debug)]
pub struct NetworkStats {
    object_count: TableGraph,
    fps: TableGraph,
    net_input: TableGraph,
    net_output: TableGraph,
    client_pings: BTreeMap<ClientId, TableGraph>,
    players: BTreeMap<i32, PlayerGraphs>,
    second_counter: i32,
    control_counter: i32,
    control_multiplier: Option<f32>,
    action_multiplier: Option<f32>,
}

impl Default for NetworkStats {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkStats {
    pub fn new() -> Self {
        let mut object_count = TableGraph::default();
        object_count.set_title("Object count");
        let mut fps = TableGraph::default();
        fps.set_title("FPS");
        let mut net_input = TableGraph::default();
        net_input.set_title("Network input");
        net_input.set_color(NET_INPUT_COLOR);
        let mut net_output = TableGraph::default();
        net_output.set_title("Network output");
        net_output.set_color(NET_OUTPUT_COLOR);
        Self {
            object_count,
            fps,
            net_input,
            net_output,
            client_pings: BTreeMap::new(),
            players: BTreeMap::new(),
            second_counter: 0,
            control_counter: 0,
            control_multiplier: None,
            action_multiplier: None,
        }
    }

    pub fn second_counter(&self) -> i32 {
        self.second_counter
    }

    pub fn control_counter(&self) -> i32 {
        self.control_counter
    }

    pub fn object_count_graph(&self) -> &TableGraph {
        &self.object_count
    }

    pub fn fps_graph(&self) -> &TableGraph {
        &self.fps
    }

    pub fn net_input_graph(&self) -> &TableGraph {
        &self.net_input
    }

    pub fn net_output_graph(&self) -> &TableGraph {
        &self.net_output
    }

    pub fn client_ping_graph(&self, client_id: ClientId) -> Option<&TableGraph> {
        self.client_pings.get(&client_id)
    }

    pub fn player_control_graph(&self, player_id: i32) -> Option<&TableGraph> {
        self.players.get(&player_id).map(|graphs| &graphs.controls)
    }

    pub fn player_action_graph(&self, player_id: i32) -> Option<&TableGraph> {
        self.players.get(&player_id).map(|graphs| &graphs.actions)
    }

    pub fn register_client(&mut self, client_id: ClientId, name: impl Into<String>) {
        let mut graph = TableGraph::new(DEFAULT_GRAPH_BACKLOG, self.second_counter);
        graph.set_title(name);
        graph.set_color(CLIENT_GRAPH_COLORS[client_id as usize % CLIENT_GRAPH_COLORS.len()]);
        self.client_pings.insert(client_id, graph);
    }

    pub fn remove_client(&mut self, client_id: ClientId) -> bool {
        self.client_pings.remove(&client_id).is_some()
    }

    pub fn register_player(&mut self, player_id: i32, name: impl Into<String>, graph_color: u32) {
        let name = name.into();
        let graph_color = readable_player_graph_color(graph_color);
        let mut controls = TableGraph::new(PLAYER_GRAPH_BACKLOG, self.control_counter);
        controls.set_title(name.clone());
        controls.set_color(graph_color);
        controls.set_average_time(CONTROL_GRAPH_AVERAGE);
        if let Some(multiplier) = self.control_multiplier {
            controls.set_multiplier(multiplier);
        }
        let mut actions = TableGraph::new(PLAYER_GRAPH_BACKLOG, self.control_counter);
        actions.set_title(name);
        actions.set_color(graph_color);
        actions.set_average_time(CONTROL_GRAPH_AVERAGE);
        if let Some(multiplier) = self.action_multiplier {
            actions.set_multiplier(multiplier);
        }
        self.players
            .insert(player_id, PlayerGraphs { controls, actions });
    }

    pub fn remove_player(&mut self, player_id: i32) -> bool {
        self.players.remove(&player_id).is_some()
    }

    pub fn record_frame(&mut self, object_count: i32) {
        self.object_count.record_value(object_count as f32);
    }

    pub fn record_second(
        &mut self,
        fps: i32,
        input: ProtocolRateSample,
        output: ProtocolRateSample,
        pings: impl IntoIterator<Item = ClientPingSample>,
    ) {
        self.fps.record_value(fps as f32);
        self.net_input.record_value(input.total() as f32);
        self.net_output.record_value(output.total() as f32);
        let pings = pings
            .into_iter()
            .map(|sample| (sample.client_id, sample.lag_ms.unwrap_or(0)))
            .collect::<BTreeMap<_, _>>();
        for (client_id, graph) in &mut self.client_pings {
            graph.record_value(pings.get(client_id).copied().unwrap_or(0) as f32);
        }
        self.second_counter += 1;
    }

    pub fn record_control_frame(
        &mut self,
        control_rate: i32,
        samples: impl IntoIterator<Item = PlayerControlSample>,
    ) {
        assert!(control_rate > 0, "control rate must be positive");
        let control_rate = control_rate as f32;
        let control_multiplier = 1000.0_f32 / 38.0 / control_rate;
        let action_multiplier = 1000.0_f32 / 38.0 * 60.0 / control_rate;
        self.control_multiplier = Some(control_multiplier);
        self.action_multiplier = Some(action_multiplier);
        let samples = samples
            .into_iter()
            .map(|sample| (sample.player_id, sample))
            .collect::<BTreeMap<_, _>>();
        for (player_id, graphs) in &mut self.players {
            let sample = samples
                .get(player_id)
                .copied()
                .unwrap_or(PlayerControlSample {
                    player_id: *player_id,
                    controls: 0,
                    actions: 0,
                });
            graphs.controls.set_multiplier(control_multiplier);
            graphs.actions.set_multiplier(action_multiplier);
            graphs.controls.record_value(sample.controls as f32);
            graphs.actions.record_value(sample.actions as f32);
        }
        self.control_counter += 1;
    }

    /// Refreshes every lazily averaged player series before graph queries.
    pub fn update(&mut self) {
        self.object_count.update();
        self.fps.update();
        self.net_input.update();
        self.net_output.update();
        for graph in self.client_pings.values_mut() {
            graph.update();
        }
        for graphs in self.players.values_mut() {
            graphs.controls.update();
            graphs.actions.update();
        }
    }

    /// Looks up the legacy graph alias. Call [`Self::update`] first when
    /// control or APM samples have changed.
    pub fn graph_by_name(&self, name: &str) -> Option<NetworkStatsGraph<'_>> {
        let (title, series) = if name.eq_ignore_ascii_case("oc") {
            ("Object count", vec![&self.object_count])
        } else if name.eq_ignore_ascii_case("fps") {
            ("FPS", vec![&self.fps])
        } else if name.eq_ignore_ascii_case("netio") {
            ("Network I/O", vec![&self.net_input, &self.net_output])
        } else if name.eq_ignore_ascii_case("pings") {
            ("Pings", self.client_pings.values().collect())
        } else if name.eq_ignore_ascii_case("control") {
            (
                "Control",
                self.players
                    .values()
                    .map(|graphs| &graphs.controls)
                    .collect(),
            )
        } else if name.eq_ignore_ascii_case("apm") {
            (
                "APM",
                self.players
                    .values()
                    .map(|graphs| &graphs.actions)
                    .collect(),
            )
        } else {
            return None;
        };
        Some(NetworkStatsGraph { title, series })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpp_graph_constants_are_pinned() {
        assert_eq!(DEFAULT_GRAPH_BACKLOG, 256);
        assert_eq!(PLAYER_GRAPH_BACKLOG, 5_120);
        assert_eq!(CONTROL_GRAPH_AVERAGE, 100);
        assert_eq!(FORWARD_AVERAGE_FACTOR, 4);
        assert_eq!(readable_player_graph_color(0), 0x65_6565);
        assert_eq!(readable_player_graph_color(0x00ff_ffff), 0x00ff_ffff);
    }

    #[test]
    fn default_backlog_retains_exactly_256_samples() {
        let mut graph = TableGraph::default();
        for value in 0..DEFAULT_GRAPH_BACKLOG {
            graph.record_value(value as f32);
        }
        assert_eq!((graph.start_time(), graph.end_time()), (0, 256));

        graph.record_value(256.0);
        assert_eq!((graph.start_time(), graph.end_time()), (1, 257));
        assert!(!graph.contains_time(0));
        assert_eq!(graph.value(1), 1.0);
        assert_eq!(graph.value(256), 256.0);
    }

    #[test]
    fn forward_average_matches_cpp_weighting() {
        let mut graph = TableGraph::new(8, 0);
        graph.set_average_time(4);
        for value in [1.0, 2.0] {
            graph.record_value(value);
        }
        graph.update();
        assert_eq!(graph.value(1).to_bits(), (14.0_f32 / 9.0).to_bits());

        for value in [3.0, 4.0, 5.0] {
            graph.record_value(value);
        }
        graph.update();
        let expected_bits = [
            0x3fb8_e38e,
            0x4000_0000,
            0x4028_0000,
            0x4055_5555,
            0x406a_aaab,
        ];
        for (time, expected_bits) in expected_bits.into_iter().enumerate() {
            assert_eq!(graph.value(time as i32).to_bits(), expected_bits);
        }
    }

    #[test]
    fn second_sample_records_fps_protocol_sums_and_ping_states() {
        let mut stats = NetworkStats::new();
        stats.register_client(7, "measured");
        stats.register_client(8, "disconnected");
        stats.register_client(9, "unmeasured");
        stats.record_frame(17);
        stats.record_second(
            38,
            ProtocolRateSample::new(100, 23),
            ProtocolRateSample::new(40, 2),
            [
                ClientPingSample {
                    client_id: 7,
                    lag_ms: Some(73),
                },
                ClientPingSample {
                    client_id: 8,
                    lag_ms: None,
                },
                ClientPingSample {
                    client_id: 9,
                    lag_ms: Some(-1),
                },
            ],
        );
        stats.register_client(10, "late");

        assert_eq!(stats.object_count_graph().value(0), 17.0);
        assert_eq!(stats.fps_graph().value(0), 38.0);
        assert_eq!(stats.net_input_graph().value(0), 123.0);
        assert_eq!(stats.net_output_graph().value(0), 42.0);
        assert_eq!(stats.client_ping_graph(7).unwrap().value(0), 73.0);
        assert_eq!(stats.client_ping_graph(8).unwrap().value(0), 0.0);
        assert_eq!(stats.client_ping_graph(9).unwrap().value(0), -1.0);
        assert_eq!(stats.client_ping_graph(10).unwrap().start_time(), 1);
        assert_eq!(stats.graph_by_name("NeTiO").unwrap().series_count(), 2);
        assert_eq!(stats.graph_by_name("PINGS").unwrap().series_count(), 4);
        assert!(stats.graph_by_name("missing").is_none());
    }

    #[test]
    fn protocol_rates_sum_as_i32_before_graph_cast() {
        let mut stats = NetworkStats::new();
        stats.record_second(
            0,
            ProtocolRateSample::new(16_777_217, 1),
            ProtocolRateSample::new(0, 0),
            [],
        );

        let cpp_sum = (16_777_217_i32 + 1) as f32;
        let separately_cast = 16_777_217_i32 as f32 + 1_i32 as f32;
        assert_eq!(
            stats.net_input_graph().value(0).to_bits(),
            cpp_sum.to_bits()
        );
        assert_ne!(cpp_sum.to_bits(), separately_cast.to_bits());
    }

    #[test]
    fn control_apm_scaling_uses_separate_cpp_f32_expressions() {
        let mut stats = NetworkStats::new();
        stats.record_control_frame(2, []);
        stats.register_player(3, "Clonk", 0x12_3456);
        assert_eq!(
            stats
                .player_control_graph(3)
                .unwrap()
                .multiplier()
                .to_bits(),
            0x4152_86bd
        );
        assert_eq!(
            stats.player_action_graph(3).unwrap().multiplier().to_bits(),
            0x4445_5e51
        );
        stats.record_control_frame(
            20,
            [PlayerControlSample {
                player_id: 3,
                controls: 1,
                actions: 1,
            }],
        );
        stats.update();

        let controls = stats.player_control_graph(3).unwrap();
        let actions = stats.player_action_graph(3).unwrap();
        let expected_control = 1000.0_f32 / 38.0 / 20.0;
        let expected_action = 1000.0_f32 / 38.0 * 60.0 / 20.0;
        assert_eq!(controls.start_time(), 1);
        assert_eq!(controls.backlog_length(), PLAYER_GRAPH_BACKLOG);
        assert_eq!(controls.average_time(), CONTROL_GRAPH_AVERAGE);
        assert_eq!(controls.multiplier().to_bits(), expected_control.to_bits());
        assert_eq!(actions.multiplier().to_bits(), expected_action.to_bits());
        assert_ne!(
            expected_action.to_bits(),
            (expected_control * 60.0).to_bits()
        );
        assert_eq!(controls.value(1).to_bits(), 0x3fa8_6bca);
        assert_eq!(actions.value(1).to_bits(), 0x429d_e50e);
    }

    #[test]
    fn multiplier_changes_rescale_retained_raw_counts() {
        let mut stats = NetworkStats::new();
        stats.register_player(1, "Clonk", 0);
        stats.record_control_frame(
            2,
            [PlayerControlSample {
                player_id: 1,
                controls: 2,
                actions: 3,
            }],
        );
        stats.update();
        let old_value = stats.player_control_graph(1).unwrap().value(0);

        stats.record_control_frame(
            20,
            [PlayerControlSample {
                player_id: 1,
                controls: 2,
                actions: 3,
            }],
        );
        stats.update();
        let controls = stats.player_control_graph(1).unwrap();
        assert_eq!(controls.raw_value(0), 2.0);
        assert_ne!(controls.value(0), old_value);
        assert_eq!(
            controls.value(0).to_bits(),
            (2.0_f32 * (1000.0_f32 / 38.0 / 20.0)).to_bits()
        );
    }

    #[test]
    fn graph_dump_uses_cpp_tsv_line_endings() {
        let mut graph = TableGraph::new(2, 4);
        graph.record_value(1.5);
        let mut dump = Vec::new();
        assert!(graph.write_tsv(&mut dump, true).unwrap());
        assert_eq!(dump, b"t\tv\r\n4\t1.5\r\n");
    }

    #[test]
    fn configured_dump_writes_full_blocks_and_flushes_the_tail() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "clonk-network-stats-{}-{nonce}.tsv",
            std::process::id()
        ));
        {
            let mut graph = TableGraph::new(2, 0);
            graph.set_dump_file(&path);
            graph.record_value(1.0);
            graph.record_value(2.0);
            graph.record_value(3.0);
        }

        let dump = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(path);
        assert_eq!(dump, b"t\tv\r\n0\t1\r\n1\t2\r\n1\t2\r\n2\t3\r\n");
    }
}
