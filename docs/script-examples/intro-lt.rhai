// HayateOffice 紹介LT（6分・約10枚）を丸ごと生成するスクリプト。
// 新規/空デッキで Tools → Script から実行（Ctrl+Enter）。スライドはデッキ末尾に追記され、
// まとめて 1 回の undo で取り消せます。
//
// ポイント: 共通の体裁（背景・上部アクセント帯・フッターのブランド名）は *スライドマスター*
// に一度だけ置きます。マスターの装飾は全スライドへ継承されるので、各スライドはタイトルと
// 本文を入れるだけ。マスターを 1 行直せば全スライドに反映されます。
//
// 使う関数（すべて生成 API。docs/scripting-api.md 参照）:
//   masters() / layouts()             -> マスター / レイアウト id の配列
//   placeholders(slide)               -> そのスライドのテンプレ枠（"title","body" など）
//   slide_add(layout)                 -> 新規スライド id
//   slide_set_title(slide, "text")    -> Title プレースホルダを埋める（位置・体裁はテンプレ継承）
//   slide_set_body(slide, "text")     -> Body プレースホルダを埋める（同上）
//   slide_set_placeholder(slide, kind, idx, "text") -> 汎用（kind="title"|"body"|"subtitle"…）
//   slide_set_background(entity,"#..")-> 背景（マスター/レイアウト/スライドに設定可、下位へ継承）
//   shape_add_rect(c, x,y,w,h)        -> 矩形 id（マスターに置くと全スライドの装飾になる）
//   shape_add_text(c, x,y,w,h,"...")  -> 自由配置テキスト / shape_set_font_size / shape_fill_accent1

let m = masters()[0];
let lay = layouts()[0];

// --- スライドマスターを一度だけブランディング（全スライドが継承）---
slide_set_background(m, "#ffffff");
// 上部アクセント帯
let bar = shape_add_rect(m, 0, 0, 960, 12);
shape_fill_accent1(bar);
// 下部アクセント帯 + フッターのブランド名（全スライドの隅に出る）
let foot = shape_add_rect(m, 0, 524, 960, 4);
shape_fill_accent1(foot);
let brand = shape_add_text(m, 40, 500, 400, 28, "HayateOffice");
shape_set_font_size(brand, 14);

// タイトル＋本文の1枚を作るヘルパ。位置や体裁はレイアウト/マスターのテンプレ枠に従う。
// （「New」直後のデッキはレイアウトに Title / Body のテンプレ枠を持っています）
fn titled(lay, title, body) {
    let s = slide_add(lay);
    // タイトルはプレーンテキストでOK。Title 枠の大きさ・太字はテンプレ（マスター）から継承する。
    slide_set_title(s, title);
    slide_set_body(s, body);
    s
}

// 1. タイトル
titled(lay, "HayateOffice",
    "Rust 製の軽量・高速プレゼンエディタ\n\nLightning Talk — 6分\n\n_このスライド自体が Rhai スクリプトで生成されています_");

// 2. 課題
titled(lay, "なぜ作るのか",
    "- 既存の Office は重い・起動が遅い\n- 互換性のための複雑さ\n- 数式やコードをリッチに書きにくい\n- 拡張・自動化がしづらい");

// 3. アーキテクチャ
titled(lay, "アーキテクチャ",
    "- *Rust* + *gpui*（GPU レンダリング）\n- 独自 IR（DOD: World / Entity）\n- フォーマット = 独自IR + 変換プラグイン（PPTX 等）\n- gpui 非依存のコア（テスト容易）");

// 4. Typst テキスト
titled(lay, "テキストは Typst",
    "- リスト・番号・*太字*・_斜体_ をそのまま記述\n- 見出しは `=`、数式は `$ ... $`\n\n$ sum_(i=1)^n i = (n (n+1)) / 2 $\n\n- 編集中はソース、確定でプレビュー");

// 5. PDF
titled(lay, "PDF は「本物のテキスト」",
    "- 画像だらけにしない方針\n- 文字は *選択・検索* 可能（埋め込み CID フォント）\n- 数式も glyph + ベクター線で出力\n- $ integral_0^1 x^2 dif x = 1/3 $ も選べる");

// 6. スライドマスター（このLTの体裁もマスターで作っています）
titled(lay, "スライドマスター",
    "- マスター → レイアウト → スライドの階層\n- 背景・帯・フッターはマスターに一度だけ\n- 全スライドが自動で継承\n- _この帯とフッターもマスター由来_");

// 7. リボン UI
titled(lay, "リボン UI",
    "- *File* … 保存 / PDF・PPTX・SVG 書き出し\n- *Home* … スライド追加 / 書式 / 整列\n- *Insert* … 図形・テキスト・画像\n- *Slideshow* … プレゼン開始\n- *Tools* … Script / Ask AI / コマンド");

// 8. Rhai 拡張
titled(lay, "Rhai スクリプト拡張",
    "- コマンド = 生成関数（`shape_add_text`, `slide_add` …）\n- *このLTも Rhai で生成* しました\n- 1 スクリプト = 1 undo・サンドボックス実行\n- API は自動ドキュメント化");

// 9. AI オーサリング
titled(lay, "AI オーサリング",
    "- 自然言語 → Rhai 生成 → 実行\n- 失敗したらエラーを戻して *自己修復*\n- 同じコマンド面を AI も叩く");

// 10. まとめ
titled(lay, "まとめ / Thank you",
    "- 軽量・高速（Rust + gpui）\n- Typst テキスト & 選択可能 PDF\n- マスター / レイアウトで一貫した体裁\n- *スクリプト & AI で拡張自在*\n\nありがとうございました！");

print("Generated 10 slides; brand chrome lives on the slide master.");
