//! Picture references into embedded media (DESIGN 6.10 component data).
//!
//! Image bytes are not stored inline; a [`PictureRef`] points at media by a content key
//! (e.g. a content hash) that the media store resolves. The natural size records the
//! image's intrinsic dimensions for default framing and aspect-ratio preservation.

use crate::geom::SizeEmu;
use serde::{Deserialize, Serialize};

/// Reference to embedded media by content key, with the image's natural size.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PictureRef {
    /// Content key of the embedded media (resolved by the media store).
    pub media_key: String,
    /// Intrinsic image size in EMU.
    pub natural: SizeEmu,
}
