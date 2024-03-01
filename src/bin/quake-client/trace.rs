use std::{fs::File, io::BufWriter, mem};

use bevy::prelude::*;
use richter::{
    client::trace::TraceFrame,
    common::console::{ExecResult, Registry},
};

const DEFAULT_TRACE_PATH: &'static str = "richter-trace.json";

/// Implements the `trace_begin` command.
pub fn cmd_trace_begin(_: &[&str], world: &mut World) -> ExecResult {
    let mut trace: &mut Option<Vec<TraceFrame>> = todo!();
    if trace.is_some() {
        error!("trace already in progress");
        "trace already in progress".into()
    } else {
        // start a new trace
        *trace = Some(Vec::new());
        default()
    }
}

/// Implements the `trace_end` command.
pub fn cmd_trace_end(_: &[&str], cvars: Res<Registry>) -> ExecResult {
    let mut trace: &mut Option<Vec<TraceFrame>> = todo!();
    if let Some(trace_frames) = mem::take(trace) {
        let trace_path = cvars.read_cvar::<String>("trace_path").unwrap();
        let trace_file = match File::create(&trace_path) {
            Ok(f) => f,
            Err(e) => {
                error!("Couldn't open trace file for write: {}", e);
                return format!("Couldn't open trace file for write: {}", e).into();
            }
        };

        let mut writer = BufWriter::new(trace_file);

        match serde_json::to_writer(&mut writer, &trace_frames) {
            Ok(()) => (),
            Err(e) => {
                error!("Couldn't serialize trace: {}", e);
                return format!("Couldn't serialize trace: {}", e).into();
            }
        };

        debug!("wrote {} frames to {}", trace_frames.len(), &trace_path);
        format!("wrote {} frames to {}", trace_frames.len(), &trace_path).into()
    } else {
        error!("no trace in progress");
        "no trace in progress".into()
    }
}
