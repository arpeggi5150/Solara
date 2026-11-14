//! The Instrument Rack (Ableton-style layout): eight macro knobs around an instrument track's
//! instrument, each moving one or more of the instrument's parameters between a low and a high
//! value. The macros are saved with the track; turning one writes the parameters it moves, so the
//! instrument plays the same in every layout and in a bounce.
use super::{CommandSpec, always, bad, str_param, track_param};

use crate::{Engine, Result, cmd};
use serde_json::{Value, json};
use soundcraft_model::{MACROS, MAX_MACRO_TARGETS, MAX_NAME_CHARS, MacroTarget, Rack, TrackId};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "rack.create",
            "Group into Instrument Rack",
            [],
            None,
            "{track}: eight macros around the track's instrument, the first eight of its continuous parameters mapped one each",
            always,
            create
        ),
        cmd!("rack.remove", "Remove Instrument Rack", [], None, "{track}: the instrument stays as it is", always, remove),
        cmd!("rack.macro", "Set Macro", [], None, "{track, macro: 0..7, value: 0..1}: the macro and every parameter it moves", always, set_macro),
        cmd!("rack.map", "Map to Macro", [], None, "{track, macro: 0..7, param, min?, max? (default: the parameter's range)}", always, map),
        cmd!("rack.unmap", "Unmap from Macro", [], None, "{track, macro: 0..7, param}", always, unmap),
        cmd!("rack.rename", "Rename Macro", [], None, "{track, macro: 0..7, name}", always, rename),
    ]
}

/// An instrument track with an instrument whose parameters are known, and the plugin's info.
fn instrument_track(e: &Engine, cmd: &str, p: &Value) -> Result<(TrackId, &'static soundcraft_dsp::PluginInfo)> {
    let t = track_param(e, cmd, p, "track")?.ok_or_else(|| bad(cmd, "`track` required"))?;
    let tr = e.session().track(t).ok_or_else(|| bad(cmd, "no such track"))?;
    if !tr.kind.plays_instrument() {
        return Err(bad(cmd, "an Instrument Rack goes on an instrument track"));
    }
    let id = tr.instrument.as_ref().map(|i| i.plugin.clone()).ok_or_else(|| bad(cmd, "the track has no instrument"))?;
    let info = super::clap::plugin_info(&id).ok_or_else(|| bad(cmd, format!("the parameters of `{id}` are not known")))?;
    Ok((t, info))
}

fn macro_index(cmd: &str, p: &Value) -> Result<usize> {
    let m = p.get("macro").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "`macro` 0..7 required"))?;
    usize::try_from(m).ok().filter(|m| *m < MACROS).ok_or_else(|| bad(cmd, format!("there are {MACROS} macros, 0 to {}", MACROS - 1)))
}

/// The track's rack, which must exist.
fn rack_of<'a>(s: &'a mut soundcraft_model::Session, cmd: &str, t: TrackId) -> Result<&'a mut Rack> {
    s.track_mut(t).and_then(|tr| tr.rack.as_mut()).ok_or_else(|| bad(cmd, "the track has no Instrument Rack: rack.create first"))
}

/// The parameter values a macro at `value` sets: (param, value) for each target.
fn targets_at(targets: &[MacroTarget], value: f32, info: &soundcraft_dsp::PluginInfo) -> Vec<(String, f32)> {
    targets
        .iter()
        .filter_map(|t| {
            let pi = info.params.iter().find(|pi| pi.id == t.param)?;
            Some((t.param.clone(), pi.clamp(t.min + (t.max - t.min) * value)))
        })
        .collect()
}

fn create(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "rack.create";
    let (t, info) = instrument_track(e, id, p)?;
    if e.session().track(t).is_some_and(|tr| tr.rack.is_some()) {
        return Ok(json!({"track": t}));
    }
    let current =
        |pid: &str, d: f32| e.session().track(t).and_then(|tr| tr.instrument.as_ref()).and_then(|i| i.params.get(pid).copied()).unwrap_or(d);
    let mut rack = Rack::new();
    let continuous =
        info.params.iter().filter(|pi| !matches!(pi.unit, soundcraft_dsp::Unit::Choice | soundcraft_dsp::Unit::Toggle) && pi.max > pi.min);
    for (m, pi) in rack.macros.iter_mut().zip(continuous) {
        m.name = pi.name.chars().take(MAX_NAME_CHARS).collect();
        m.targets.push(MacroTarget { param: pi.id.to_string(), min: pi.min, max: pi.max });
        let v = current(pi.id, pi.default);
        m.value = ((v - pi.min) / (pi.max - pi.min)).clamp(0.0, 1.0);
    }
    if let Some(tr) = e.session_mut().track_mut(t) {
        tr.rack = Some(rack);
    }
    Ok(json!({"track": t}))
}

fn remove(e: &mut Engine, p: &Value) -> Result<Value> {
    let t = track_param(e, "rack.remove", p, "track")?.ok_or_else(|| bad("rack.remove", "`track` required"))?;
    if e.session().track(t).is_some_and(|tr| tr.rack.is_some())
        && let Some(tr) = e.session_mut().track_mut(t)
    {
        tr.rack = None;
    }
    Ok(json!({"track": t}))
}

fn set_macro(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "rack.macro";
    let (t, info) = instrument_track(e, id, p)?;
    let m = macro_index(id, p)?;
    let v = p.get("value").and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| bad(id, "`value` 0..1 required"))?;
    let v = v.clamp(0.0, 1.0) as f32;
    let s = e.session_mut();
    let rack = rack_of(s, id, t)?;
    let Some(mac) = rack.macros.get_mut(m) else { return Err(bad(id, "no such macro")) };
    mac.value = v;
    let sets = targets_at(&mac.targets, v, info);
    if let Some(ins) = s.track_mut(t).and_then(|tr| tr.instrument.as_mut()) {
        for (k, x) in &sets {
            ins.params.insert(k.clone(), *x);
        }
    }
    Ok(json!({"macro": m, "value": v, "params": sets.len()}))
}

fn map(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "rack.map";
    let (t, info) = instrument_track(e, id, p)?;
    let m = macro_index(id, p)?;
    let param = str_param(p, "param").ok_or_else(|| bad(id, "`param` required"))?;
    let pi = info.params.iter().find(|pi| pi.id == param).ok_or_else(|| bad(id, format!("`{}` has no parameter `{param}`", info.id)))?;
    let f = |k: &str, d: f32| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).map_or(d, |v| pi.clamp(v as f32));
    let (min, max) = (f("min", pi.min), f("max", pi.max));
    let s = e.session_mut();
    let rack = rack_of(s, id, t)?;
    let Some(mac) = rack.macros.get_mut(m) else { return Err(bad(id, "no such macro")) };
    mac.targets.retain(|x| x.param != param);
    if mac.targets.len() >= MAX_MACRO_TARGETS {
        return Err(bad(id, format!("a macro moves at most {MAX_MACRO_TARGETS} parameters")));
    }
    mac.targets.push(MacroTarget { param: param.to_string(), min, max });
    Ok(json!({"macro": m, "targets": mac.targets.len()}))
}

fn unmap(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "rack.unmap";
    let t = track_param(e, id, p, "track")?.ok_or_else(|| bad(id, "`track` required"))?;
    let m = macro_index(id, p)?;
    let param = str_param(p, "param").ok_or_else(|| bad(id, "`param` required"))?.to_string();
    let rack = rack_of(e.session_mut(), id, t)?;
    let Some(mac) = rack.macros.get_mut(m) else { return Err(bad(id, "no such macro")) };
    mac.targets.retain(|x| x.param != param);
    Ok(json!({"macro": m, "targets": mac.targets.len()}))
}

fn rename(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "rack.rename";
    let t = track_param(e, id, p, "track")?.ok_or_else(|| bad(id, "`track` required"))?;
    let m = macro_index(id, p)?;
    let name: String = str_param(p, "name").ok_or_else(|| bad(id, "`name` required"))?.chars().take(MAX_NAME_CHARS).collect();
    let rack = rack_of(e.session_mut(), id, t)?;
    if let Some(mac) = rack.macros.get_mut(m) {
        mac.name = name;
    }
    Ok(json!({"macro": m}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use soundcraft_model::{ChannelFormat, TrackKind};

    #[test]
    fn eight_macros_move_the_instrument_inside() {
        let mut e = Engine::default();
        let s = e.session_mut();
        let t = s.add_track(TrackKind::Instrument, ChannelFormat::Stereo, None);
        s.track_mut(t).unwrap().instrument = Some(soundcraft_model::Insert::new("subtractive_synth"));
        e.execute("rack.create", &json!({"track": t.0})).unwrap();
        let rack = e.session().track(t).unwrap().rack.clone().unwrap();
        assert_eq!(rack.macros.len(), 8);
        assert!(rack.macros.iter().all(|m| m.targets.len() == 1), "{rack:?}");
        let target = rack.macros[0].targets[0].clone();
        e.execute("rack.macro", &json!({"track": t.0, "macro": 0, "value": 1.0})).unwrap();
        let v = |e: &Engine| e.session().track(t).unwrap().instrument.as_ref().unwrap().params.get(&target.param).copied();
        assert_eq!(v(&e), Some(target.max));
        e.execute("rack.macro", &json!({"track": t.0, "macro": 0, "value": 0.0})).unwrap();
        assert_eq!(v(&e), Some(target.min));
        // One macro can move a second parameter over a narrower range.
        let other = rack.macros[1].targets[0].param.clone();
        e.execute(
            "rack.map",
            &json!({"track": t.0, "macro": 0, "param": other, "min": rack.macros[1].targets[0].min, "max": rack.macros[1].targets[0].min}),
        )
        .unwrap();
        e.execute("rack.macro", &json!({"track": t.0, "macro": 0, "value": 0.5})).unwrap();
        let ins = e.session().track(t).unwrap().instrument.clone().unwrap();
        assert_eq!(ins.params.get(&other).copied(), Some(rack.macros[1].targets[0].min));
        assert!(e.execute("rack.macro", &json!({"track": t.0, "macro": 8, "value": 0.5})).is_err());
        assert!(e.execute("rack.map", &json!({"track": t.0, "macro": 0, "param": "nope"})).is_err());
        let midi = e.session_mut().add_track(TrackKind::Midi, ChannelFormat::Mono, None);
        assert!(e.execute("rack.create", &json!({"track": midi.0})).is_err());
    }
}
