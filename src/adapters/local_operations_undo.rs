use std::rc::Rc;
use gtk::{gio, glib, prelude::*};
use crate::model::FileEntry;
use crate::services::{OperationEvent, OperationProvider, OperationRequestId, UndoEntry};

// This file is just for drafting the implementation
