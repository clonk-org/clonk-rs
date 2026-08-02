use crate::math::{fixed100, FixedVec2};
use crate::{Engine, EngineError, Vector2};

/// The shared deterministic portion of C4Object::UpdateInLiquid and Splash.
/// The engine and compatibility-host paths provide the live-world operations,
/// but the probe, entry gate, random draw order, surface scan, and extraction
/// order must remain one implementation (C4Object.cpp:6093-6110;
/// C4Effect.cpp:801-835).
pub(crate) trait SplashHost {
    type Error;

    fn splash_is_semi_solid(&self, x: i32, y: i32) -> bool;
    fn splash_material_is_liquid(&self, x: i32, y: i32) -> bool;
    fn splash_is_liquid(&self, x: i32, y: i32) -> bool;
    fn splash_random(&mut self, upper_bound: i32) -> Result<i32, Self::Error>;
    fn splash_bubble_out(&mut self, x: i32, y: i32) -> Result<(), Self::Error>;
    fn splash_extract_and_cast(
        &mut self,
        source: Vector2,
        destination: Vector2,
        velocity: FixedVec2,
    ) -> Result<(), Self::Error>;
}

pub(crate) fn liquid_probe_y(position_y: i32, float_line: i32, construction: i32) -> i32 {
    position_y.saturating_add(
        float_line
            .saturating_mul(construction)
            .checked_div(crate::FULL_CON)
            .unwrap_or(0),
    ) - 1
}

pub(crate) fn entered_liquid(wet: bool, was_in_liquid: bool) -> bool {
    wet && !was_in_liquid
}

pub(crate) fn should_splash(wet: bool, was_in_liquid: bool, ocf: u32, mass: i32) -> bool {
    entered_liquid(wet, was_in_liquid) && ocf & crate::ocf::HIT_SPEED2 != 0 && mass > 3
}

pub(crate) fn splash_amount(width: i32, height: i32) -> i32 {
    (width.saturating_mul(height) / 10).min(20)
}

pub(crate) fn run_splash<H: SplashHost>(
    host: &mut H,
    tx: i32,
    ty: i32,
    amount: i32,
) -> Result<(), H::Error> {
    if host.splash_is_semi_solid(tx, ty - 15) || !host.splash_material_is_liquid(tx, ty) {
        return Ok(());
    }

    let mut surface_y = ty;
    while host.splash_is_liquid(tx, surface_y) && surface_y > ty - 20 && surface_y >= 0 {
        surface_y -= 1;
    }

    for _ in 0..amount {
        // Keep C++'s explicit r2/r1 evaluation order (C4Effect.cpp:815-819).
        let r2 = host.splash_random(16)?;
        let r1 = host.splash_random(16)?;
        host.splash_bubble_out(tx + r1 - 8, ty + r2 - 6)?;
        if host.splash_is_liquid(tx, ty) && !host.splash_is_semi_solid(tx, surface_y) {
            // Keep C++'s r2/r1 order before ExtractMaterial
            // (C4Effect.cpp:820-829).
            let r2 = -host.splash_random(200)?;
            let r1 = host.splash_random(151)? - 75;
            host.splash_extract_and_cast(
                Vector2::new(tx, ty),
                Vector2::new(tx, surface_y),
                FixedVec2::new(fixed100(r1), fixed100(r2)),
            )?;
        }
    }
    // C++ StartSoundEffect (C4Effect.cpp:833-835) is presentation-only and
    // does not participate in the synchronized simulation state.
    Ok(())
}

impl SplashHost for Engine {
    type Error = EngineError;

    fn splash_is_semi_solid(&self, x: i32, y: i32) -> bool {
        self.landscape
            .as_ref()
            .is_some_and(|landscape| landscape.is_semi_solid_at(x, y))
    }

    fn splash_material_is_liquid(&self, x: i32, y: i32) -> bool {
        self.landscape
            .as_ref()
            .and_then(|landscape| landscape.material_at(x, y))
            .and_then(|id| self.materials.get_by_id(id))
            .is_some_and(|material| (25..50).contains(&material.density()) && material.instable())
    }

    fn splash_is_liquid(&self, x: i32, y: i32) -> bool {
        self.landscape
            .as_ref()
            .is_some_and(|landscape| landscape.is_liquid_at(x, y))
    }

    fn splash_random(&mut self, upper_bound: i32) -> Result<i32, Self::Error> {
        Ok(self.rng.random(upper_bound))
    }

    fn splash_bubble_out(&mut self, x: i32, y: i32) -> Result<(), Self::Error> {
        self.bubble_out(x, y)
    }

    fn splash_extract_and_cast(
        &mut self,
        source: Vector2,
        destination: Vector2,
        velocity: FixedVec2,
    ) -> Result<(), Self::Error> {
        if let Some(material) = self.extract_material(source.x, source.y) {
            self.pxs_system.create(
                material,
                crate::math::itofix(destination.x),
                crate::math::itofix(destination.y),
                velocity.x,
                velocity.y,
            );
        }
        Ok(())
    }
}
