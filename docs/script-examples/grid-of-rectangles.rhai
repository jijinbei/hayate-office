// Build a 3x4 grid of rectangles on the current slide.
let s = current_slide();
for row in 0..3 {
    for col in 0..4 {
        let x = 40 + col * 120;
        let y = 40 + row * 100;
        shape_add_rect(s, x, y, 100, 80);
    }
}
