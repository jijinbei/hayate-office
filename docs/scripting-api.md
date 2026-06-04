# HayateOffice scripting-API reference

GENERATED FILE — do not edit by hand. Regenerate with `REGEN_DOCS=1 cargo test -p hayate-core scripting_api`.

## Shape

| Command | Params | Description |
| --- | --- | --- |
| `shape.delete` | `entity`: entity | Delete the target shape (despawn the entity). |
| `shape.set_fill` | `entity`: entity, `color`: color | Set the shape fill to a solid color. |
| `shape.fill_gradient` | `entity`: entity, `from`: color, `to`: color, `angle`: float | Set the shape fill to a two-stop linear gradient. |
| `shape.move` | `entity`: entity, `dx`: int, `dy`: int | Translate the shape's frame by (dx, dy). |
| `shape.set_position` | `entity`: entity, `x`: float, `y`: float | Set the shape's frame origin to an absolute point (in points), keeping its size. |
| `shape.set_size` | `entity`: entity, `w`: float, `h`: float | Set the shape's frame size to an absolute size (in points), keeping its origin. |
| `shape.bring_to_front` | `entity`: entity | Move the shape to the front (largest Order) among its siblings. |
| `shape.send_to_back` | `entity`: entity | Move the shape to the back (smallest Order) among its siblings. |
| `shape.set_rotation` | `entity`: entity, `degrees`: float | Set the shape's rotation (degrees clockwise). |
| `shape.reset_rotation` | `entity`: entity | Clear any rotation by setting it back to 0 degrees. |
| `shape.rotate_by` | `entity`: entity, `degrees`: float | Add the given degrees to the shape's current rotation. |

## Style

| Command | Params | Description |
| --- | --- | --- |
| `shape.set_opacity` | `entity`: entity, `value`: float | Set the shape's opacity (clamped to 0.0..=1.0). |
| `shape.fill_accent1` | `entity`: entity | Set the shape fill to theme accent color 1. |
| `shape.fill_accent2` | `entity`: entity | Set the shape fill to theme accent color 2. |
| `shape.fill_accent3` | `entity`: entity | Set the shape fill to theme accent color 3. |
| `shape.fill_accent4` | `entity`: entity | Set the shape fill to theme accent color 4. |
| `shape.fill_accent5` | `entity`: entity | Set the shape fill to theme accent color 5. |
| `shape.fill_accent6` | `entity`: entity | Set the shape fill to theme accent color 6. |
| `shape.fill_black` | `entity`: entity | Set the shape fill to literal black. |
| `shape.fill_white` | `entity`: entity | Set the shape fill to literal white. |

## Text

| Command | Params | Description |
| --- | --- | --- |
| `shape.set_text` | `entity`: entity, `text`: string | Set the shape's text to a single string, preserving the first run's formatting. |
| `shape.set_font_size` | `entity`: entity, `pt`: float | Set every run's font size to the given points (minimum 1pt). |
| `shape.set_font` | `entity`: entity, `family`: string | Set every run's font family to the given name. |
| `shape.toggle_bold` | `entity`: entity | Toggle bold across the whole text box. |
| `shape.toggle_italic` | `entity`: entity | Toggle italic across the whole text box. |
| `shape.toggle_underline` | `entity`: entity | Toggle underline across the whole text box. |
| `shape.align_text_left` | `entity`: entity | Set every paragraph's horizontal alignment to left. |
| `shape.align_text_center` | `entity`: entity | Set every paragraph's horizontal alignment to center. |
| `shape.align_text_right` | `entity`: entity | Set every paragraph's horizontal alignment to right. |

## Arrange

| Command | Params | Description |
| --- | --- | --- |
| `shapes.align_left` | `entities`: entity | Align the selected shapes' left edges to the group bounding box. |
| `shapes.align_hcenter` | `entities`: entity | Align the selected shapes' horizontal centers to the group bounding box. |
| `shapes.align_right` | `entities`: entity | Align the selected shapes' right edges to the group bounding box. |
| `shapes.align_top` | `entities`: entity | Align the selected shapes' top edges to the group bounding box. |
| `shapes.align_vcenter` | `entities`: entity | Align the selected shapes' vertical centers to the group bounding box. |
| `shapes.align_bottom` | `entities`: entity | Align the selected shapes' bottom edges to the group bounding box. |
| `shapes.distribute_h` | `entities`: entity | Evenly distribute the selected shapes along the horizontal axis. |
| `shapes.distribute_v` | `entities`: entity | Evenly distribute the selected shapes along the vertical axis. |
