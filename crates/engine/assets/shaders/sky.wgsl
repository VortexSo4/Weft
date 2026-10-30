// The sky behind the world: one full screen triangle drawn first in the world pass, never
// tested against the depth, so everything else covers it.
//
// From the horizon, where it is the fog's color so distant terrain meets it, to the dimension's
// sky color overhead; a square sun and moon drawn on a pixel grid like the blocks, stars that
// turn with them at night, a red glow around a low sun, and blocky clouds drifting over it all.
// `sky.params.y` (weather) grows the clouds into an overcast layer and hides what is behind it.

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u)) * 2.0 - 1.0;
    var out: VOut;
    out.clip = vec4(p, 0.0, 1.0);
    out.ndc = p;
    return out;
}

fn hash(x: u32) -> u32 {
    var h = x * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return (h >> 22u) ^ h;
}

// 0..1 for a lattice point.
fn hash2(p: vec2<i32>) -> f32 {
    return f32(hash(bitcast<u32>(p.x) ^ hash(bitcast<u32>(p.y) + 0x9e3779b9u)) & 0xffffffu) / 16777216.0;
}

fn hash3(p: vec3<i32>, salt: u32) -> f32 {
    return f32(hash(bitcast<u32>(p.x) ^ hash(bitcast<u32>(p.y) ^ hash(bitcast<u32>(p.z) + salt))) & 0xffffffu) / 16777216.0;
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash2(i);
    let b = hash2(i + vec2(1, 0));
    let c = hash2(i + vec2(0, 1));
    let d = hash2(i + vec2(1, 1));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var s = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        s += value_noise(q) * amp;
        q = q * 2.03 + vec2(17.3, 9.1);
        amp *= 0.5;
    }
    return s / 0.9375;
}

// Where `dir` meets the square of a body at `toward` with half size `size`: -1..1 across it on
// both axes, outside that range when it misses.
fn on_square(dir: vec3<f32>, toward: vec3<f32>, size: f32) -> vec2<f32> {
    let facing = dot(dir, toward);
    if facing <= 0.0 {
        return vec2(9.0);
    }
    let across = normalize(cross(toward, vec3(0.0, 0.0, 1.0)));
    let up = cross(across, toward);
    let q = dir / facing - toward;
    return vec2(dot(q, across), dot(q, up)) / size;
}

// The sun: a bright square of 8 by 8 pixels with a paler rim.
fn sun_disc(sq: vec2<f32>) -> vec4<f32> {
    if max(abs(sq.x), abs(sq.y)) > 1.0 {
        return vec4(0.0);
    }
    let px = vec2<i32>(floor((sq * 0.5 + 0.5) * 8.0));
    let rim = px.x == 0 || px.y == 0 || px.x == 7 || px.y == 7;
    let c = select(vec3(1.0, 0.97, 0.78), vec3(1.0, 0.82, 0.42), rim);
    return vec4(c, 1.0);
}

// The moon: a pale square of 8 by 8 pixels with darker craters.
fn moon_disc(sq: vec2<f32>) -> vec4<f32> {
    if max(abs(sq.x), abs(sq.y)) > 1.0 {
        return vec4(0.0);
    }
    let px = vec2<i32>(floor((sq * 0.5 + 0.5) * 8.0));
    let crater = hash2(px + vec2(31, 7)) < 0.22;
    let rim = px.x == 0 || px.y == 0 || px.x == 7 || px.y == 7;
    var c = vec3(0.86, 0.88, 0.96);
    if crater {
        c = vec3(0.62, 0.65, 0.76);
    }
    if rim {
        c *= 0.85;
    }
    return vec4(c, 1.0);
}

fn stars(dir: vec3<f32>, sun: vec3<f32>, time: f32) -> f32 {
    // The sky turns with the sun, around the axis its path circles (z).
    let a = atan2(sun.y, sun.x);
    let cs = cos(-a);
    let sn = sin(-a);
    let d = vec3(dir.x * cs - dir.y * sn, dir.x * sn + dir.y * cs, dir.z);
    let p = d * 160.0;
    let cell = vec3<i32>(floor(p));
    if hash3(cell, 1u) > 0.014 {
        return 0.0;
    }
    let at = vec3<f32>(cell) + 0.5 + (vec3(hash3(cell, 2u), hash3(cell, 3u), hash3(cell, 4u)) - 0.5) * 0.6;
    let glow = smoothstep(0.22, 0.0, length(p - at));
    let twinkle = 0.75 + 0.25 * sin(time * (1.5 + 3.0 * hash3(cell, 5u)) + hash3(cell, 6u) * 6.28);
    return glow * (0.45 + 0.55 * hash3(cell, 7u)) * twinkle;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let horizon = sky.horizon.rgb;
    // A dimension without sky light (the void) is all fog.
    if sky.params.w < 0.5 {
        return vec4(horizon, 1.0);
    }
    let far = sky.inv_view_proj * vec4(in.ndc, 1.0, 1.0);
    let dir = normalize(far.xyz / far.w);
    let day = sky.params.x;
    let overcast = sky.params.y;
    let time = sky.params.z;
    let sun = normalize(scene.sun.xyz);
    let up = dir.y;
    let clear = 1.0 - overcast;

    // The gradient: fog at and below the horizon, quickly giving way to the sky color.
    var c = mix(horizon, sky.zenith.rgb, pow(clamp(up, 0.0, 1.0), 0.45));

    // Around the sun the sky is lighter by day; low suns burn the horizon red on their side.
    c += vec3(1.0, 0.9, 0.7) * pow(max(dot(dir, sun), 0.0), 6.0) * 0.12 * day * clear;
    let low = 1.0 - smoothstep(0.0, 0.45, abs(sun.y));
    let side = pow(max(dot(dir, normalize(vec3(sun.x, max(sun.y, 0.0) * 0.5, sun.z))), 0.0), 3.0);
    let band = 1.0 - smoothstep(-0.05, 0.5, abs(up));
    c = mix(c, vec3(1.0, 0.4, 0.14) * max(day, 0.3) * 1.2, low * side * band * 0.75 * (1.0 - 0.8 * overcast));

    // Bodies and stars only above the horizon; below it the terrain's fog would be.
    let above = smoothstep(-0.02, 0.03, up);
    let night = 1.0 - smoothstep(0.0, 0.35, day);
    c += vec3(0.9, 0.92, 1.0) * stars(dir, sun, time) * night * clear * smoothstep(0.0, 0.2, up);
    let s = sun_disc(on_square(dir, sun, 0.075));
    let glow = pow(max(dot(dir, sun), 0.0), 900.0) * 0.6 + pow(max(dot(dir, sun), 0.0), 120.0) * 0.25;
    c = mix(c, s.rgb, s.a * above * clear);
    c += vec3(1.0, 0.85, 0.55) * glow * above * clear;
    let m = moon_disc(on_square(dir, -sun, 0.055));
    let moon_glow = pow(max(dot(dir, -sun), 0.0), 600.0) * 0.25;
    c = mix(c, m.rgb * mix(0.35, 1.0, night), m.a * above * clear);
    c += vec3(0.6, 0.7, 1.0) * moon_glow * above * clear * night;

    // Clouds: a flat layer far overhead, in blocks of the noise like the world's, thicker and
    // grayer the more overcast it is.
    if up > 0.0 {
        let uv = dir.xz / max(up, 0.02);
        let wind = vec2(time * 0.012, time * 0.004);
        let cell = floor((uv + wind) * 5.0);
        let n = fbm(cell * 0.11);
        // A storm closes the layer.
        let cover = mix(0.6, 0.0, overcast);
        let fade = smoothstep(0.02, 0.35, up);
        if n > cover {
            // White by day, at night only a little lighter than the sky they hide the stars of;
            // under weather a layer a shade darker than the grayed fog.
            let lit = mix(0.025, 1.0, smoothstep(0.0, 0.6, day));
            var cloud = mix(vec3(lit), horizon * 0.85, overcast);
            // Their undersides catch the evening red too.
            cloud = mix(cloud, vec3(1.0, 0.55, 0.35) * lit, low * side * 0.5 * clear);
            // Inner cells a little darker than the edges, so one cloud still reads as a shape.
            let core = smoothstep(cover, cover + 0.25, n);
            cloud *= 1.0 - 0.12 * core;
            c = mix(c, cloud, fade * mix(0.85, 0.97, overcast));
        }
    }
    return vec4(c, 1.0);
}
