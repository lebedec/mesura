use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

pub struct Counter {
    state: Box<State>,
}

impl Counter {
    pub fn new(name: &str) -> Self {
        Self::with_labels(name, [], [0; 0])
    }

    pub fn with_labels<const N: usize>(
        name: &str,
        keys: [&str; N],
        labels: [impl ToString; N],
    ) -> Self {
        let labels = format_labels(keys, labels.map(|v| v.to_string()));
        let mut registry = get_metrics().write().expect("registry must be valid");
        let counter = State {
            name: name.to_string(),
            key: format!("{name}{labels}"),
            help: String::new(),
            value: Default::default(),
            kind: Kind::Counter,
        };
        let mut state = Box::new(counter);
        registry.register(&mut state);
        Self { state }
    }

    pub fn inc(&mut self) {
        self.add(1);
    }

    pub fn add(&mut self, value: usize) {
        self.state.value += value as f64;
    }

    pub fn value(&self) -> usize {
        self.state.value as usize
    }
}

pub struct Gauge {
    state: Box<State>,
}

impl Gauge {
    pub fn new(name: &str) -> Self {
        Self::with_labels(name, [], [0; 0])
    }

    pub fn with_labels<const N: usize>(
        name: &str,
        keys: [&str; N],
        labels: [impl ToString; N],
    ) -> Self {
        let labels = format_labels(keys, labels.map(|v| v.to_string()));
        let mut registry = get_metrics().write().expect("registry must be valid");
        let counter = State {
            name: name.to_string(),
            key: format!("{name}{labels}"),
            help: String::new(),
            value: Default::default(),
            kind: Kind::Gauge,
        };
        let mut state = Box::new(counter);
        registry.register(&mut state);
        Self { state }
    }

    pub fn value(&self) -> f64 {
        self.state.value
    }
}

pub trait GaugeValue<T> {
    fn set(&mut self, value: T);
    fn add(&mut self, value: T);
}

impl GaugeValue<Instant> for Gauge {
    fn set(&mut self, value: Instant) {
        self.state.value = value.elapsed().as_secs_f64();
    }

    fn add(&mut self, value: Instant) {
        self.state.value += value.elapsed().as_secs_f64();
    }
}

impl GaugeValue<&mut Stopwatch> for Gauge {
    fn set(&mut self, value: &mut Stopwatch) {
        self.state.value = value.lap().elapsed().as_secs_f64();
    }

    fn add(&mut self, value: &mut Stopwatch) {
        self.state.value += value.lap().elapsed().as_secs_f64();
    }
}

impl GaugeValue<usize> for Gauge {
    fn set(&mut self, value: usize) {
        self.state.value = value as f64;
    }

    fn add(&mut self, value: usize) {
        self.state.value += value as f64;
    }
}

impl GaugeValue<i32> for Gauge {
    fn set(&mut self, value: i32) {
        self.state.value = value as f64;
    }

    fn add(&mut self, value: i32) {
        self.state.value += value as f64;
    }
}

impl GaugeValue<f32> for Gauge {
    fn set(&mut self, value: f32) {
        self.state.value = value as f64;
    }

    fn add(&mut self, value: f32) {
        self.state.value += value as f64;
    }
}

enum Kind {
    Counter,
    Gauge,
}

struct State {
    name: String,
    key: String,
    help: String,
    value: f64,
    kind: Kind,
}

fn format_labels<const N: usize>(keys: [&str; N], labels: [String; N]) -> String {
    if N > 0 {
        let pairs: Vec<String> = keys
            .iter()
            .zip(labels)
            .map(|(key, label)| format!("{key}=\"{label}\""))
            .collect();
        let pairs = pairs.join(",").to_string();
        format!("{{{pairs}}}")
    } else {
        String::new()
    }
}

impl Drop for State {
    fn drop(&mut self) {
        let mut registry = get_metrics().write().expect("registry must be valid");
        registry.unregister(&self);
    }
}

pub fn get_metrics() -> &'static RwLock<Registry> {
    static SINGLETON: OnceLock<RwLock<Registry>> = OnceLock::new();
    SINGLETON.get_or_init(|| RwLock::new(Registry::new()))
}

pub struct Registry {
    metrics: BTreeMap<String, AtomicPtr<State>>,
}

impl Registry {
    pub fn new() -> Self {
        Registry {
            metrics: Default::default(),
        }
    }

    fn register(&mut self, state: &mut Box<State>) {
        let ptr = AtomicPtr::new(state.as_mut());
        self.metrics.insert(state.key.clone(), ptr);
    }

    fn unregister(&mut self, state: &State) {
        if let Some(record) = self.metrics.get(&state.key) {
            if record.load(Ordering::Relaxed) as *const _ != state as *const _ {
                // new metric state with same key overwrites record via registration
                // nothing to do, old state dropped
                return;
            } else {
                self.metrics.remove(&state.key);
            }
        }
    }

    pub fn encode_prometheus_report(&self) -> String {
        let mut current = String::new();
        let mut output = String::new();
        for ptr in self.metrics.values() {
            let ptr = ptr.load(Ordering::Relaxed);
            // SAFETY: pointer is valid because metrics on drop removed from registry via RwLock
            let metric = unsafe { &*ptr };
            let name = &metric.name;
            let value = &metric.value;
            let key = &metric.key;
            if metric.name != current {
                let kind = match metric.kind {
                    Kind::Counter => "counter",
                    Kind::Gauge => "gauge",
                };
                if !metric.help.is_empty() {
                    output += &format!("# HELP {name} {}\n", metric.help);
                }
                output += &format!("# TYPE {name} {kind}\n");
                current = name.clone();
            }
            output += &format!("{key} {value}\n");
        }
        output
    }
}

struct MyMetrics {
    metric_a: Counter,
    update_time: Gauge,
    metric_b: Counter,
}

pub fn test_usage() {
    {
        let mut m = MyMetrics {
            metric_a: Counter::with_labels("metric", ["key"], ["a"]),
            update_time: Gauge::new("metric_c"),
            metric_b: Counter::with_labels("metric", ["key", "container"], ["b", "my_service"]),
        };
        loop {
            let time = Instant::now();
            println!("usage");
            m.metric_a.inc();
            m.metric_b.inc();
            sleep(Duration::from_millis(16));
            m.update_time.set(time);
            println!(
                "a {} b {} c {}",
                m.metric_a.value(),
                m.metric_b.value(),
                m.update_time.value(),
            );
            if m.metric_a.value() > 100 {
                break;
            }
            sleep(Duration::from_millis(16));
        }
    }
    sleep(Duration::from_secs(1));
    println!("bye");
}

pub fn test_thread() {
    loop {
        println!("gather");
        let report = {
            // NOTE: minimize lock in scope
            let registry = get_metrics()
                .read()
                .expect("registry must be valid to read");
            registry.encode_prometheus_report()
        };
        println!("REPORT:\n{report}");
        sleep(Duration::from_secs(1));
    }
}

pub struct Stopwatch {
    timestamp: Instant,
}

impl Stopwatch {
    pub fn new() -> Self {
        Stopwatch {
            timestamp: Instant::now(),
        }
    }

    pub fn lap(&mut self) -> Instant {
        let value = self.timestamp;
        self.timestamp = Instant::now();
        value
    }
}

// # HELP dt ...
// # TYPE dt histogram
// dt_bucket{le="0.008"} 4
// dt_bucket{le="0.016"} 4
// dt_bucket{le="0.033"} 4
// dt_bucket{le="0.066"} 4
// dt_bucket{le="+Inf"} 4
// dt_sum 0.000263737
// dt_count 4
// # HELP gauge2 ...
// # TYPE gauge2 gauge
// gauge2{a="biba",b="buba"} 4
// # HELP text_render_count ...
// # TYPE text_render_count counter
// text_render_count 4
// # HELP text_render_keys_count ...
// # TYPE text_render_keys_count gauge
// text_render_keys_count{key="buba"} 4
//
