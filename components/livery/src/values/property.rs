// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use std::{fmt, str::FromStr};

use super::{
    ComputedColor, Length, LengthPercentage, MathLengthPercentage, Matrix2D, ParseError,
    RelativeLengthEnvironment, UsedColorContext, format_number, keyword_value,
};

mod animation;
mod backgrounds;
mod columns;
mod containment;
mod layout;
mod transforms;
mod typography;

// Every value type keeps its `values::` path: the families are an internal
// arrangement, not a new public surface.
pub use animation::*;
pub use backgrounds::*;
pub use columns::*;
pub use containment::*;
pub use layout::*;
pub use transforms::*;
pub use typography::*;
