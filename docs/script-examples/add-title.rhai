// Add a title text box at the top of the current slide.
let s = current_slide();
let t = shape_add_text(s, 40, 20, 600, 80, "Generated title");
shape_set_font_size(t, 40);
