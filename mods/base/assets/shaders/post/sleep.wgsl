// Lying in bed: the picture fades toward black as sleep comes.
fn apply(p: PostPixel) -> vec4<f32> {
    return vec4(p.color.rgb * (1.0 - 0.85 * p.strength), 1.0);
}
