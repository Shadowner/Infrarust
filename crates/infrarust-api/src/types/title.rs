use super::Component;

#[derive(Debug, Clone, PartialEq)]
pub struct TitleData {
    pub title: Component,
    pub subtitle: Component,
    pub fade_in_ticks: i32,
    pub stay_ticks: i32,
    pub fade_out_ticks: i32,
}

impl TitleData {
    #[must_use]
    pub const fn new(title: Component, subtitle: Component) -> Self {
        Self {
            title,
            subtitle,
            fade_in_ticks: 10,
            stay_ticks: 70,
            fade_out_ticks: 20,
        }
    }

    #[must_use]
    pub const fn fade_in(mut self, ticks: i32) -> Self {
        self.fade_in_ticks = ticks;
        self
    }

    #[must_use]
    pub const fn stay(mut self, ticks: i32) -> Self {
        self.stay_ticks = ticks;
        self
    }

    #[must_use]
    pub const fn fade_out(mut self, ticks: i32) -> Self {
        self.fade_out_ticks = ticks;
        self
    }
}
