// Nudge every shape on the current slide 10pt to the right.
for e in shapes(current_slide()) {
    shape_move(e, 10, 0);
}
