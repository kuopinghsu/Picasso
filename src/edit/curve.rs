//! Tone curves. Point curves are monotone cubics through their points, so a
//! curve never inverts tones; the parametric curve is one built from four
//! region amounts and three movable splits.

/// Curve values at `size` evenly spaced inputs over 0…1.
pub fn sample(points: &[[f32; 2]], size: usize) -> Vec<f32> {
    let mut points: Vec<[f32; 2]> = points
        .iter()
        .filter(|[x, y]| x.is_finite() && y.is_finite())
        .map(|[x, y]| [x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)])
        .collect();
    points.sort_by(|a, b| a[0].total_cmp(&b[0]));
    points.dedup_by(|later, earlier| (later[0] - earlier[0]).abs() < 1e-4);
    let at = |index: usize| index as f32 / (size.max(2) - 1) as f32;
    if points.len() < 2 {
        return (0..size).map(at).collect();
    }
    let n = points.len();
    let slopes: Vec<f32> = points
        .windows(2)
        .map(|pair| (pair[1][1] - pair[0][1]) / (pair[1][0] - pair[0][0]))
        .collect();
    let mut tangents = vec![0.0f32; n];
    tangents[0] = slopes[0];
    tangents[n - 1] = slopes[n - 2];
    for k in 1..n - 1 {
        tangents[k] = if slopes[k - 1] * slopes[k] <= 0.0 {
            0.0
        } else {
            (slopes[k - 1] + slopes[k]) * 0.5
        };
    }
    // Fritsch–Carlson: limit the tangents so each piece stays monotone.
    for k in 0..n - 1 {
        if slopes[k] == 0.0 {
            tangents[k] = 0.0;
            tangents[k + 1] = 0.0;
            continue;
        }
        let a = tangents[k] / slopes[k];
        let b = tangents[k + 1] / slopes[k];
        let length = a * a + b * b;
        if length > 9.0 {
            let t = 3.0 / length.sqrt();
            tangents[k] = t * a * slopes[k];
            tangents[k + 1] = t * b * slopes[k];
        }
    }
    (0..size)
        .map(|index| {
            let x = at(index);
            if x <= points[0][0] {
                return points[0][1];
            }
            if x >= points[n - 1][0] {
                return points[n - 1][1];
            }
            let k = points.partition_point(|point| point[0] <= x).saturating_sub(1).min(n - 2);
            let [x0, y0] = points[k];
            let [x1, y1] = points[k + 1];
            let h = x1 - x0;
            let t = (x - x0) / h;
            let (t2, t3) = (t * t, t * t * t);
            let y = (2.0 * t3 - 3.0 * t2 + 1.0) * y0
                + (t3 - 2.0 * t2 + t) * h * tangents[k]
                + (-2.0 * t3 + 3.0 * t2) * y1
                + (t3 - t2) * h * tangents[k + 1];
            y.clamp(0.0, 1.0)
        })
        .collect()
}

/// Anchor points of the parametric curve. `amounts` are shadows, darks,
/// lights, and highlights in −1…1; `splits` are the region borders in 0…1.
pub fn parametric_points(amounts: [f32; 4], splits: [f32; 3]) -> Vec<[f32; 2]> {
    let [s1, s2, s3] = splits;
    let centers = [s1 * 0.5, (s1 + s2) * 0.5, (s2 + s3) * 0.5, (s3 + 1.0) * 0.5];
    let mut points = vec![[0.0, 0.0]];
    let mut floor = 0.0f32;
    for (x, amount) in centers.into_iter().zip(amounts) {
        let y = (x + amount * 0.22).clamp(0.0, 1.0).max(floor);
        floor = y;
        points.push([x, y]);
    }
    points.push([1.0, 1.0]);
    points
}

/// Linear interpolation into a table sampled over 0…1.
pub fn lookup(table: &[f32], x: f32) -> f32 {
    let last = table.len().saturating_sub(1);
    if last == 0 {
        return table.first().copied().unwrap_or(x);
    }
    let position = x.clamp(0.0, 1.0) * last as f32;
    let index = (position as usize).min(last - 1);
    let fraction = position - index as f32;
    table[index] + (table[index + 1] - table[index]) * fraction
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_pass_their_points_and_stay_monotone() {
        let points = [[0.0, 0.0], [0.25, 0.15], [0.75, 0.9], [1.0, 1.0]];
        let table = sample(&points, 1001);
        assert!((table[250] - 0.15).abs() < 1e-3);
        assert!((table[750] - 0.9).abs() < 1e-3);
        assert!(table.windows(2).all(|pair| pair[1] >= pair[0] - 1e-6));
        let identity = sample(&[[0.0, 0.0], [1.0, 1.0]], 11);
        assert!((identity[3] - 0.3).abs() < 1e-5);
        let anchors = parametric_points([1.0, 1.0, -1.0, -1.0], [0.25, 0.5, 0.75]);
        assert!(anchors.windows(2).all(|pair| pair[1][1] >= pair[0][1]));
    }
}
