// SPDX-License-Identifier: GPL-3.0-or-later

//! The Linux worker protocol depends on Linux descriptor and socket ABIs.
//! Pending platforms reject requests until their isolated implementation exists.

use super::{Cancellation, ParseOperation};
use std::path::Path;

pub(crate) fn default_worker_limit() -> usize {
    1
}

pub(crate) fn run() -> Result<(), String> {
    crate::platform::preview_isolation().require_implemented()
}

type PreviewReply = Option<Result<(Vec<u8>, Option<Vec<u8>>), String>>;

pub(crate) fn preview(
    _input: &Path,
    _operation: &ParseOperation,
    _cancellation: &Cancellation,
) -> PreviewReply {
    Some(
        crate::platform::preview_isolation()
            .require_implemented()
            .map(|()| (Vec::new(), None)),
    )
}

pub(crate) fn thumbnail_parse(
    _input: &Path,
    _operation: ParseOperation,
    _cancellation: &Cancellation,
) -> Option<Result<Vec<u8>, String>> {
    Some(
        crate::platform::preview_isolation()
            .require_implemented()
            .map(|()| Vec::new()),
    )
}
