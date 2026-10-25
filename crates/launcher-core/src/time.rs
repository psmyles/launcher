//! Local wall-clock time, for log stamps and run-history folder names.

/// A local calendar time, to the millisecond.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub millis: u16,
}

impl LocalTime {
    /// `HH:MM:SS.mmm`.
    pub fn clock(&self) -> String {
        format!(
            "{:02}:{:02}:{:02}.{:03}",
            self.hour, self.minute, self.second, self.millis
        )
    }

    /// `YYYY-MM-DD HH:MM:SS`.
    pub fn iso(&self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    /// `YYYY-MM-DD_HH-MM-SS`, safe in a file name and sortable.
    pub fn file_stamp(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

#[cfg(windows)]
pub fn now() -> LocalTime {
    // SAFETY: GetLocalTime only fills the returned struct.
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    LocalTime {
        year: t.wYear,
        month: t.wMonth as u8,
        day: t.wDay as u8,
        hour: t.wHour as u8,
        minute: t.wMinute as u8,
        second: t.wSecond as u8,
        millis: t.wMilliseconds,
    }
}

#[cfg(unix)]
pub fn now() -> LocalTime {
    // SAFETY: clock_gettime and localtime_r only fill the structs passed to them.
    unsafe {
        let mut ts: libc::timespec = std::mem::zeroed();
        libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts);
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&ts.tv_sec, &mut tm);
        LocalTime {
            year: (tm.tm_year + 1900) as u16,
            month: (tm.tm_mon + 1) as u8,
            day: tm.tm_mday as u8,
            hour: tm.tm_hour as u8,
            minute: tm.tm_min as u8,
            second: tm.tm_sec as u8,
            millis: (ts.tv_nsec / 1_000_000) as u16,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        let t = LocalTime {
            year: 2026,
            month: 9,
            day: 3,
            hour: 7,
            minute: 5,
            second: 9,
            millis: 42,
        };
        assert_eq!(t.clock(), "07:05:09.042");
        assert_eq!(t.iso(), "2026-09-03 07:05:09");
        assert_eq!(t.file_stamp(), "2026-09-03_07-05-09");
        let n = now();
        assert!(n.year >= 2024 && (1..=12).contains(&n.month));
    }
}
