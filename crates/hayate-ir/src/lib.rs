//! HayateOffice ドキュメントモデルの基本型（gpui非依存・純データ）。
//!
//! ここはデータ型のみを持ち、編集ロジック（操作・Undo）や描画には依存しない。
//! 設計は `docs/DESIGN.md` を参照。

pub mod color;
pub mod frac;
pub mod geom;
pub mod units;
