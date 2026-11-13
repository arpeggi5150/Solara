//! Elsewhere than 64-bit Windows there are no drivers to host.

use crate::{DriverCaps, Notify, Opened, Render, Request};

pub(crate) struct Stream;

pub(crate) fn drivers() -> Vec<String> {
    Vec::new()
}

pub(crate) fn probe(name: &str) -> Result<DriverCaps, String> {
    Err(format!("no driver named `{name}`: low-latency drivers are hosted on 64-bit Windows only"))
}

pub(crate) fn open(request: &Request, _render: Render, _notify: Notify) -> Result<(Stream, Opened), String> {
    Err(format!("cannot open `{}`: low-latency drivers are hosted on 64-bit Windows only", request.driver))
}
