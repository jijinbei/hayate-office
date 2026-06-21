# HayateOffice scripting-API reference

GENERATED FILE — do not edit by hand. Regenerate with `REGEN_DOCS=1 cargo test -p hayate-core scripting_api`.

Each command is exposed to Rhai scripts as the listed function (positional arguments in the shown order). Argument types: `entity` = integer id, `color` = string (`"#RRGGBB"` or a theme token like `"accent1"`), `int`/`float` = number, `bool`, `string`. The read-only helper `entities()` returns the ids of all shapes. A whole script is applied as one undoable change.

## Shape

| Script call | Command id | Description |
| --- | --- | --- |
| `shape_delete(entity)` | `shape.delete` | Delete the target shape (despawn the entity). |
| `shape_set_fill(entity, color)` | `shape.set_fill` | Set the shape fill to a solid color. |
| `shape_fill_gradient(entity, from, to, angle)` | `shape.fill_gradient` | Set the shape fill to a two-stop linear gradient. |
| `shape_move(entity, dx, dy)` | `shape.move` | Translate the shape's frame by (dx, dy). |
| `shape_set_position(entity, x, y)` | `shape.set_position` | Set the shape's frame origin to an absolute point (in points), keeping its size. |
| `shape_set_size(entity, w, h)` | `shape.set_size` | Set the shape's frame size to an absolute size (in points), keeping its origin. |
| `shape_bring_to_front(entity)` | `shape.bring_to_front` | Move the shape to the front (largest Order) among its siblings. |
| `shape_send_to_back(entity)` | `shape.send_to_back` | Move the shape to the back (smallest Order) among its siblings. |
| `shape_set_rotation(entity, degrees)` | `shape.set_rotation` | Set the shape's rotation (degrees clockwise). |
| `shape_reset_rotation(entity)` | `shape.reset_rotation` | Clear any rotation by setting it back to 0 degrees. |
| `shape_rotate_by(entity, degrees)` | `shape.rotate_by` | Add the given degrees to the shape's current rotation. |

## Style

| Script call | Command id | Description |
| --- | --- | --- |
| `shape_set_opacity(entity, value)` | `shape.set_opacity` | Set the shape's opacity (clamped to 0.0..=1.0). |
| `shape_fill_accent1(entity)` | `shape.fill_accent1` | Set the shape fill to theme accent color 1. |
| `shape_fill_accent2(entity)` | `shape.fill_accent2` | Set the shape fill to theme accent color 2. |
| `shape_fill_accent3(entity)` | `shape.fill_accent3` | Set the shape fill to theme accent color 3. |
| `shape_fill_accent4(entity)` | `shape.fill_accent4` | Set the shape fill to theme accent color 4. |
| `shape_fill_accent5(entity)` | `shape.fill_accent5` | Set the shape fill to theme accent color 5. |
| `shape_fill_accent6(entity)` | `shape.fill_accent6` | Set the shape fill to theme accent color 6. |
| `shape_fill_black(entity)` | `shape.fill_black` | Set the shape fill to literal black. |
| `shape_fill_white(entity)` | `shape.fill_white` | Set the shape fill to literal white. |

## Text

| Script call | Command id | Description |
| --- | --- | --- |
| `shape_set_text(entity, text)` | `shape.set_text` | Set the shape's text to a single string, preserving the first run's formatting. |
| `shape_set_font_size(entity, pt)` | `shape.set_font_size` | Set every run's font size to the given points (minimum 1pt). |
| `shape_set_font(entity, family)` | `shape.set_font` | Set every run's font family to the given name. |
| `shape_toggle_bold(entity)` | `shape.toggle_bold` | Toggle bold across the whole text box. |
| `shape_toggle_italic(entity)` | `shape.toggle_italic` | Toggle italic across the whole text box. |
| `shape_toggle_underline(entity)` | `shape.toggle_underline` | Toggle underline across the whole text box. |
| `shape_align_text_left(entity)` | `shape.align_text_left` | Set every paragraph's horizontal alignment to left. |
| `shape_align_text_center(entity)` | `shape.align_text_center` | Set every paragraph's horizontal alignment to center. |
| `shape_align_text_right(entity)` | `shape.align_text_right` | Set every paragraph's horizontal alignment to right. |

## Arrange

| Script call | Command id | Description |
| --- | --- | --- |
| `shapes_align_left(entities)` | `shapes.align_left` | Align the selected shapes' left edges to the group bounding box. |
| `shapes_align_hcenter(entities)` | `shapes.align_hcenter` | Align the selected shapes' horizontal centers to the group bounding box. |
| `shapes_align_right(entities)` | `shapes.align_right` | Align the selected shapes' right edges to the group bounding box. |
| `shapes_align_top(entities)` | `shapes.align_top` | Align the selected shapes' top edges to the group bounding box. |
| `shapes_align_vcenter(entities)` | `shapes.align_vcenter` | Align the selected shapes' vertical centers to the group bounding box. |
| `shapes_align_bottom(entities)` | `shapes.align_bottom` | Align the selected shapes' bottom edges to the group bounding box. |
| `shapes_distribute_h(entities)` | `shapes.distribute_h` | Evenly distribute the selected shapes along the horizontal axis. |
| `shapes_distribute_v(entities)` | `shapes.distribute_v` | Evenly distribute the selected shapes along the vertical axis. |

## Create

| Script call | Command id | Description |
| --- | --- | --- |
| `shape_add_rect(parent, x, y, w, h)` | `shape.add_rect` | Create a rectangle on a slide/group at (x, y) with size (w, h), in points. |
| `shape_add_ellipse(parent, x, y, w, h)` | `shape.add_ellipse` | Create an ellipse on a slide/group at (x, y) with size (w, h), in points. |
| `shape_add_text(parent, x, y, w, h, text)` | `shape.add_text` | Create a text box on a slide/group at (x, y) with size (w, h) in points and the given text. |

## Slide

| Script call | Command id | Description |
| --- | --- | --- |
| `slide_add(layout)` | `slide.add` | Create a new slide using the given layout; returns the new slide id. |
| `slide_set_background(slide, color)` | `slide.set_background` | Set the slide's background to a solid color (#RRGGBB or a theme token). |
