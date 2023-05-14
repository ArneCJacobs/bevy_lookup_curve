use bevy_math::Vec2;
use bevy_reflect::{Reflect, FromReflect, TypeUuid};

pub mod editor;

#[derive(Reflect, FromReflect, Copy, Clone, Debug)]
pub enum KnotInterpolation {
  Constant,
  Linear,
  Bezier,
}

#[derive(Reflect, FromReflect, Copy, Clone, Debug)]
pub struct Knot {
  /// The position of this knot in curve space
  pub position: Vec2,

  /// Interpolation used between this and the next knot
  pub interpolation: KnotInterpolation,

  /// Identifier used by editor operations because index might change during modification
  pub id: usize,
  
  /// Left tangent relative to knot position. x above 0 will be clamped to 0
  pub left_tangent: Vec2,
  /// Right tangent relative to knot position. x below 0 will be clamped to 0
  pub right_tangent: Vec2,
}

impl Knot {
  // TODO: Refactor repetitive *_corrected-implementations

  /// Returns the left tangent of this knot, corrected between the previous knot and this one.
  /// Ensures the curve does not ever go backwards.
  fn left_tangent_corrected(&self, prev_knot: Option<&Knot>) -> Vec2 {
    if self.left_tangent.x >= 0.0 {
      return Vec2::new(0.0, self.left_tangent.y);
    }

    if let Some(prev_knot) = prev_knot {
      let min_x = prev_knot.position.x - self.position.x;
      if self.left_tangent.x < min_x {
        return Vec2::new(min_x, self.left_tangent.y * (min_x / self.left_tangent.x));
      }
    }

    self.left_tangent
  }

  /// Returns the right tangent of this knot, corrected between the next knot and this one.
  /// Ensures the curve does not ever go backwards.
  fn right_tangent_corrected(&self, next_knot: Option<&Knot>) -> Vec2 {
    if self.right_tangent.x <= 0.0 {
      return Vec2::new(0.0, self.right_tangent.y);
    }

    if let Some(next_knot) = next_knot {
      let max_x = next_knot.position.x - self.position.x;
      if self.right_tangent.x > max_x {
        return Vec2::new(max_x, self.right_tangent.y * (max_x / self.right_tangent.x));
      }
    }

    self.right_tangent
  }
}

impl Default for Knot {
  fn default() -> Self {
    Self {
      position: Vec2::ZERO,
      interpolation: KnotInterpolation::Linear,
      id: 0,
      right_tangent: Vec2::new(0.1, 0.0),
      left_tangent: Vec2::new(-0.1, 0.0),
    }
  }
}

/// Two-dimensional spline that only allows a single y-value per x-value
#[derive(Debug, TypeUuid, Reflect, FromReflect)]
#[uuid = "3219b5f0-fff6-42fd-9fc8-fd98ff8dae35"]
pub struct LookupCurve {
  knots: Vec<Knot>,
}

impl LookupCurve {
  pub fn new(mut knots: Vec<Knot>) -> Self {
    knots.sort_by(|a, b|
      a.position.x
        .partial_cmp(&b.position.x)
        .expect("NaN is not allowed")
    );
    
    Self {
      knots,
    }
  }

  pub fn knots(&self) -> &[Knot] {
    self.knots.as_slice()
  }

  /// Modifies an existing knot in the lookup curve. Returns the new (possibly unchanged) index of the knot.
  fn modify_knot(&mut self, i: usize, new_value: &Knot) -> usize {
    let old_value = self.knots[i];
    if old_value.position == new_value.position {
      // The knot has not been moved, simply overwrite it
      self.knots[i] = *new_value;
      return i;
    }

    // binary seach for new idx
    let new_i = self.knots.partition_point(|knot| knot.position.x < new_value.position.x);
    if new_i == i {
      // knot stays in the same spot even though position was changed, overwrite it
      self.knots[i] = *new_value;
      return i;
    }

    self.knots.remove(i);

    let insert_i = if i < new_i { new_i - 1 } else { new_i };
    self.knots.insert(insert_i, *new_value);

    insert_i
  }

  /// Deletes a knot given index
  fn delete_knot(&mut self, i: usize) {
    self.knots.remove(i);
  }

  /// Find y given x
  pub fn find_y_given_x(&self, x: f32) -> f32 {
    // Return repeated constant values outside of knot range
    if self.knots.is_empty() {
      return 0.0;
    }
    if self.knots.len() == 1 || x <= self.knots[0].position.x {
      return self.knots[0].position.y;
    }
    if x >= self.knots[self.knots.len() - 1].position.x {
      return self.knots[self.knots.len() - 1].position.y;
    }

    // Find left knot
    let i = self.knots.partition_point(|knot| knot.position.x < x) - 1;
    let knot_a = self.knots[i];

    // Interpolate
    match knot_a.interpolation {
      KnotInterpolation::Constant => knot_a.position.y,
      KnotInterpolation::Linear => {
        let knot_b = &self.knots[i+1];
        let s = (x - knot_a.position.x) / (knot_b.position.x - knot_a.position.x);
        knot_a.position.lerp(knot_b.position, s).y
      },
      KnotInterpolation::Bezier => {
        let knot_b = &self.knots[i+1];
        // TODO: Optimize (we only need to calculate the coefficients when the knot is added/modified)
        CubicSegment::from_bezier_points([
          knot_a.position,
          knot_a.position + knot_a.right_tangent_corrected(Some(knot_b)),
          knot_b.position + knot_b.left_tangent_corrected(Some(&knot_a)),
          knot_b.position,
        ]).find_y_given_x(x)
      }
    }
  }
}

/// Mostly a copy of code from https://github.com/bevyengine/bevy/blob/main/crates/bevy_math/src/cubic_splines.rs
/// 
/// Copied because the cubic_splines module does not exactly fit the API we need:
/// 1. Allow constructing a single CubicSegment from bezier points (without allocating a cubiccurve and without restricting c0 and c1 to 0 and 1)
/// 2. find_y_given_x needs to be accessible
#[derive(Clone, Debug, Default, PartialEq)]
struct CubicSegment{
  coeff: [Vec2; 4],
}

impl CubicSegment {
  /// Instantaneous position of a point at parametric value `t`.
  #[inline]
  pub fn position(&self, t: f32) -> Vec2 {
    let [a, b, c, d] = self.coeff;
    a + b * t + c * t.powi(2) + d * t.powi(3)
  }

  /// Instantaneous velocity of a point at parametric value `t`.
  #[inline]
  pub fn velocity(&self, t: f32) -> Vec2 {
    let [_, b, c, d] = self.coeff;
    b + c * 2.0 * t + d * 3.0 * t.powi(2)
  }

  #[inline]
  fn find_y_given_x(&self, x: f32) -> f32 {
    const MAX_ERROR: f32 = 1e-5;
    const MAX_ITERS: u8 = 8;
  
    let mut t_guess = x;
    let mut pos_guess = Vec2::ZERO;
    for _ in 0..MAX_ITERS {
      pos_guess = self.position(t_guess);
      let error = pos_guess.x - x;
      if error.abs() <= MAX_ERROR {
          break;
      }
      // Using Newton's method, use the tangent line to estimate a better guess value.
      let slope = self.velocity(t_guess).x; // dx/dt
      t_guess -= error / slope;
    }
    pos_guess.y
  }

  #[inline]
  fn from_bezier_points(control_points: [Vec2; 4]) -> CubicSegment {
    let char_matrix = [
      [1., 0., 0., 0.],
      [-3., 3., 0., 0.],
      [3., -6., 3., 0.],
      [-1., 3., -3., 1.],
    ];

    Self::coefficients(control_points, 1.0, char_matrix)
  }

  #[inline]
  fn coefficients(p: [Vec2; 4], multiplier: f32, char_matrix: [[f32; 4]; 4]) -> CubicSegment {
    let [c0, c1, c2, c3] = char_matrix;
    // These are the polynomial coefficients, computed by multiplying the characteristic
    // matrix by the point matrix.
    let mut coeff = [
      p[0] * c0[0] + p[1] * c0[1] + p[2] * c0[2] + p[3] * c0[3],
      p[0] * c1[0] + p[1] * c1[1] + p[2] * c1[2] + p[3] * c1[3],
      p[0] * c2[0] + p[1] * c2[1] + p[2] * c2[2] + p[3] * c2[3],
      p[0] * c3[0] + p[1] * c3[1] + p[2] * c3[2] + p[3] * c3[3],
    ];
    coeff.iter_mut().for_each(|c| *c *= multiplier);
    CubicSegment { coeff }
  }
}
