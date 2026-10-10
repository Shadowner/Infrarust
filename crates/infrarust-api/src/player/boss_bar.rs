use std::fmt;
use std::sync::Arc;

use uuid::Uuid;

use crate::error::PlayerError;
use crate::types::Component;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum BossBarColor {
    #[default]
    Pink,
    Blue,
    Red,
    Green,
    Yellow,
    Purple,
    White,
}

impl BossBarColor {
    pub const fn id(self) -> i32 {
        match self {
            Self::Pink => 0,
            Self::Blue => 1,
            Self::Red => 2,
            Self::Green => 3,
            Self::Yellow => 4,
            Self::Purple => 5,
            Self::White => 6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum BossBarOverlay {
    #[default]
    Progress,
    Notched6,
    Notched10,
    Notched12,
    Notched20,
}

impl BossBarOverlay {
    pub const fn id(self) -> i32 {
        match self {
            Self::Progress => 0,
            Self::Notched6 => 1,
            Self::Notched10 => 2,
            Self::Notched12 => 3,
            Self::Notched20 => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BossBarFlags(u8);

impl BossBarFlags {
    pub const DARKEN_SCREEN: u8 = 0x01;
    pub const PLAY_BOSS_MUSIC: u8 = 0x02;
    pub const CREATE_WORLD_FOG: u8 = 0x04;
    pub const NONE: Self = Self(0);

    pub const fn new(bits: u8) -> Self {
        Self(bits & 0x07)
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, flag: u8) -> bool {
        self.0 & flag == flag
    }

    #[must_use]
    pub const fn with(self, flag: u8) -> Self {
        Self::new(self.0 | flag)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BossBar {
    pub title: Component,
    pub progress: f32,
    pub color: BossBarColor,
    pub overlay: BossBarOverlay,
    pub flags: BossBarFlags,
}

impl BossBar {
    pub fn new(title: Component) -> Self {
        Self {
            title,
            progress: 1.0,
            color: BossBarColor::default(),
            overlay: BossBarOverlay::default(),
            flags: BossBarFlags::NONE,
        }
    }

    #[must_use]
    pub const fn progress(mut self, progress: f32) -> Self {
        self.progress = progress;
        self
    }

    #[must_use]
    pub const fn color(mut self, color: BossBarColor) -> Self {
        self.color = color;
        self
    }

    #[must_use]
    pub const fn overlay(mut self, overlay: BossBarOverlay) -> Self {
        self.overlay = overlay;
        self
    }

    #[must_use]
    pub const fn flags(mut self, flags: BossBarFlags) -> Self {
        self.flags = flags;
        self
    }

    pub fn apply(&mut self, update: &BossBarUpdate) {
        match update {
            BossBarUpdate::Title(title) => self.title = title.clone(),
            BossBarUpdate::Progress(progress) => self.progress = *progress,
            BossBarUpdate::Style { color, overlay } => {
                self.color = *color;
                self.overlay = *overlay;
            }
            BossBarUpdate::Flags(flags) => self.flags = *flags,
        }
    }
}

pub fn clamp_progress(progress: f32) -> f32 {
    if progress.is_nan() {
        0.0
    } else {
        progress.clamp(0.0, 1.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum BossBarUpdate {
    Title(Component),
    Progress(f32),
    Style {
        color: BossBarColor,
        overlay: BossBarOverlay,
    },
    Flags(BossBarFlags),
}

pub trait BossBarControl: Send + Sync {
    fn update(&self, id: Uuid, update: BossBarUpdate) -> Result<(), PlayerError>;

    fn hide(&self, id: Uuid) -> Result<(), PlayerError>;
}

#[derive(Clone)]
pub struct BossBarHandle {
    id: Uuid,
    control: Arc<dyn BossBarControl>,
}

impl BossBarHandle {
    pub fn new(id: Uuid, control: Arc<dyn BossBarControl>) -> Self {
        Self { id, control }
    }

    pub const fn id(&self) -> Uuid {
        self.id
    }

    pub fn update(&self, update: BossBarUpdate) -> Result<(), PlayerError> {
        self.control.update(self.id, update)
    }

    pub fn set_title(&self, title: Component) -> Result<(), PlayerError> {
        self.update(BossBarUpdate::Title(title))
    }

    pub fn set_progress(&self, progress: f32) -> Result<(), PlayerError> {
        self.update(BossBarUpdate::Progress(progress))
    }

    pub fn set_style(
        &self,
        color: BossBarColor,
        overlay: BossBarOverlay,
    ) -> Result<(), PlayerError> {
        self.update(BossBarUpdate::Style { color, overlay })
    }

    pub fn set_flags(&self, flags: BossBarFlags) -> Result<(), PlayerError> {
        self.update(BossBarUpdate::Flags(flags))
    }

    pub fn hide(&self) -> Result<(), PlayerError> {
        self.control.hide(self.id)
    }
}

impl fmt::Debug for BossBarHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BossBarHandle")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct Recording {
        calls: Mutex<Vec<(Uuid, Option<BossBarUpdate>)>>,
    }

    impl BossBarControl for Recording {
        fn update(&self, id: Uuid, update: BossBarUpdate) -> Result<(), PlayerError> {
            self.calls.lock().unwrap().push((id, Some(update)));
            Ok(())
        }

        fn hide(&self, id: Uuid) -> Result<(), PlayerError> {
            self.calls.lock().unwrap().push((id, None));
            Ok(())
        }
    }

    #[test]
    fn the_handle_forwards_to_its_control() {
        let control = Arc::new(Recording::default());
        let id = Uuid::from_u128(1);
        let handle = BossBarHandle::new(id, Arc::clone(&control) as Arc<dyn BossBarControl>);
        handle.set_progress(0.5).unwrap();
        handle
            .set_style(BossBarColor::Red, BossBarOverlay::Notched10)
            .unwrap();
        handle.clone().hide().unwrap();
        let calls = control.calls.lock().unwrap();
        assert_eq!(
            *calls,
            vec![
                (id, Some(BossBarUpdate::Progress(0.5))),
                (
                    id,
                    Some(BossBarUpdate::Style {
                        color: BossBarColor::Red,
                        overlay: BossBarOverlay::Notched10,
                    })
                ),
                (id, None),
            ]
        );
    }

    #[test]
    fn updates_apply_to_the_description() {
        let mut bar = BossBar::new(Component::text("a"));
        bar.apply(&BossBarUpdate::Title(Component::text("b")));
        bar.apply(&BossBarUpdate::Flags(
            BossBarFlags::NONE.with(BossBarFlags::DARKEN_SCREEN),
        ));
        assert_eq!(bar.title, Component::text("b"));
        assert!(bar.flags.contains(BossBarFlags::DARKEN_SCREEN));
        assert!(!bar.flags.contains(BossBarFlags::CREATE_WORLD_FOG));
    }

    #[test]
    fn progress_is_clamped_and_ids_follow_the_protocol() {
        assert!((clamp_progress(1.5) - 1.0).abs() < f32::EPSILON);
        assert!(clamp_progress(-1.0).abs() < f32::EPSILON);
        assert!(clamp_progress(f32::NAN).abs() < f32::EPSILON);
        assert_eq!(BossBarColor::White.id(), 6);
        assert_eq!(BossBarOverlay::Notched20.id(), 4);
        assert_eq!(BossBarFlags::new(0xFF).bits(), 0x07);
    }
}
