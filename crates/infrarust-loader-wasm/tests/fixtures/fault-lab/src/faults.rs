pub const FAULTS_FILE: &str = "faults.txt";
pub const LOG_FILE: &str = "log.txt";
pub const CODEC_FILTER_ID: &str = "fault-lab-codec";
pub const LIMBO_HANDLER: &str = "fault-lab-limbo";
pub const CODEC_FAULT_CONNECTION_BASE: u64 = 9000;
pub const CODEC_FAULT_PACKET_BASE: i32 = 0x70;
pub const CODEC_MARK_PACKET: i32 = 0x05;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Panic,
    Unreachable,
    Stack,
    Grow,
    Huge,
    Spin,
    Sleep,
    Refuse,
    Linger,
}

impl Mode {
    pub const TRAPS: [Self; 7] = [
        Self::Panic,
        Self::Unreachable,
        Self::Stack,
        Self::Grow,
        Self::Huge,
        Self::Spin,
        Self::Sleep,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Panic => "panic",
            Self::Unreachable => "unreachable",
            Self::Stack => "stack",
            Self::Grow => "grow",
            Self::Huge => "huge",
            Self::Spin => "spin",
            Self::Sleep => "sleep",
            Self::Refuse => "refuse",
            Self::Linger => "linger",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        [
            Self::Panic,
            Self::Unreachable,
            Self::Stack,
            Self::Grow,
            Self::Huge,
            Self::Spin,
            Self::Sleep,
            Self::Refuse,
            Self::Linger,
        ]
        .into_iter()
        .find(|mode| mode.as_str() == word)
    }

    #[allow(dead_code)]
    pub fn code(self) -> u64 {
        Self::TRAPS
            .iter()
            .position(|mode| *mode == self)
            .map_or(u64::MAX, |at| at as u64)
    }

    pub fn from_code(code: u64) -> Option<Self> {
        usize::try_from(code)
            .ok()
            .and_then(|at| Self::TRAPS.get(at).copied())
    }
}

pub fn mode_at(faults: &str, site: &str) -> Option<Mode> {
    faults.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next() == Some(site))
            .then(|| words.next().and_then(Mode::parse))
            .flatten()
    })
}

pub fn directives(faults: &str) -> Vec<Vec<String>> {
    faults
        .lines()
        .map(|line| {
            line.split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|words| !words.is_empty())
        .collect()
}
