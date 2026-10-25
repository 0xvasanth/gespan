//! A daily log file, `gespan-native.YYYY-MM-DD.jsonl`, with retention: keep
//! 7 days or 50 MB in total, whichever limit is hit first.
use chrono::NaiveDate;
use std::{
    fs::File,
    io::{self, Write},
    path::{Path, PathBuf},
};

pub const MAX_DAYS: i64 = 7;
pub const MAX_BYTES: u64 = 50 * 1024 * 1024;
const PREFIX: &str = "gespan-native.";
const SUFFIX: &str = ".jsonl";

pub trait Clock: Send + 'static {
    fn today(&self) -> NaiveDate;
}

/// Days roll over at UTC midnight, matching the log lines' UTC timestamps.
pub struct SystemClock;
impl Clock for SystemClock {
    fn today(&self) -> NaiveDate {
        chrono::Utc::now().date_naive()
    }
}

pub fn file_name(day: NaiveDate) -> String {
    format!("{PREFIX}{}{SUFFIX}", day.format("%Y-%m-%d"))
}

/// Our daily files in `dir`, oldest first. Anything else there (crash files,
/// `.DS_Store`, other names) is never touched.
fn scan(dir: &Path) -> io::Result<Vec<(NaiveDate, PathBuf, u64)>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(day) = name
            .to_str()
            .and_then(|name| name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX))
            .and_then(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").ok())
        else {
            continue;
        };
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_file() {
            files.push((day, entry.path(), metadata.len()));
        }
    }
    files.sort();
    Ok(files)
}

/// Deletes daily files older than `max_days`, then the oldest until the total
/// fits `max_bytes`. Today's file is never deleted. Returns what was deleted.
pub fn prune(
    dir: &Path,
    today: NaiveDate,
    max_days: i64,
    max_bytes: u64,
) -> io::Result<Vec<PathBuf>> {
    let files = scan(dir)?;
    let oldest_kept = today - chrono::Duration::days(max_days - 1);
    let mut total: u64 = files.iter().map(|(_, _, size)| size).sum();
    let mut deleted = Vec::new();
    for (day, path, size) in files {
        if day == today {
            continue;
        }
        if day < oldest_kept || total > max_bytes {
            std::fs::remove_file(&path)?;
            total -= size;
            deleted.push(path);
        }
    }
    Ok(deleted)
}

pub struct DailyFile<C: Clock> {
    dir: PathBuf,
    clock: C,
    max_days: i64,
    max_bytes: u64,
    day: NaiveDate,
    file: File,
    /// Bytes in today's file.
    size: u64,
    /// Bytes in the other days' files, as of the last prune.
    others: u64,
    /// Today's share of the budget is spent; drop lines until tomorrow.
    full: bool,
}

impl<C: Clock> DailyFile<C> {
    /// Creates the folder, prunes it and opens today's file for appending.
    pub fn new(dir: PathBuf, clock: C, max_days: i64, max_bytes: u64) -> io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        let day = clock.today();
        let (file, size) = Self::open(&dir, day)?;
        let mut this = Self {
            dir,
            clock,
            max_days,
            max_bytes,
            day,
            file,
            size,
            others: 0,
            full: false,
        };
        this.prune(0)?;
        Ok(this)
    }

    fn open(dir: &Path, day: NaiveDate) -> io::Result<(File, u64)> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(file_name(day)))?;
        let size = file.metadata()?.len();
        Ok((file, size))
    }

    /// Prunes so that `incoming` more bytes still fit, and recounts the rest.
    fn prune(&mut self, incoming: u64) -> io::Result<()> {
        prune(
            &self.dir,
            self.day,
            self.max_days,
            self.max_bytes.saturating_sub(incoming),
        )?;
        self.others = scan(&self.dir)?
            .into_iter()
            .filter(|(day, _, _)| *day != self.day)
            .map(|(_, _, size)| size)
            .sum();
        Ok(())
    }

    fn fits(&self, len: u64) -> bool {
        self.size + self.others + len <= self.max_bytes
    }
}

impl<C: Clock> Write for DailyFile<C> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let today = self.clock.today();
        if today != self.day {
            self.file.flush()?;
            self.day = today;
            (self.file, self.size) = Self::open(&self.dir, today)?;
            self.full = false;
            self.prune(0)?;
        }
        let len = buf.len() as u64;
        if !self.full && !self.fits(len) {
            self.prune(len)?;
            self.full = !self.fits(len);
        }
        if self.full {
            // Report success: a full log must not turn into write errors.
            return Ok(buf.len());
        }
        self.file.write_all(buf)?;
        self.size += len;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct FakeClock(Arc<Mutex<NaiveDate>>);
    impl FakeClock {
        fn at(day: NaiveDate) -> Self {
            Self(Arc::new(Mutex::new(day)))
        }
        fn advance(&self, days: i64) {
            let mut day = self.0.lock().unwrap();
            *day += chrono::Duration::days(days);
        }
    }
    impl Clock for FakeClock {
        fn today(&self) -> NaiveDate {
            *self.0.lock().unwrap()
        }
    }

    fn day(n: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, n).unwrap()
    }
    fn seed(dir: &Path, day: NaiveDate, bytes: usize) -> PathBuf {
        let path = dir.join(file_name(day));
        std::fs::write(&path, vec![b'x'; bytes]).unwrap();
        path
    }
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn names_files_by_day() {
        assert_eq!(file_name(day(5)), "gespan-native.2026-10-05.jsonl");
    }

    #[test]
    fn writes_to_todays_file_and_rolls_at_midnight() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FakeClock::at(day(24));
        let mut file =
            DailyFile::new(dir.path().into(), clock.clone(), MAX_DAYS, MAX_BYTES).unwrap();
        file.write_all(b"{\"a\":1}\n").unwrap();
        clock.advance(1);
        file.write_all(b"{\"b\":2}\n").unwrap();
        file.flush().unwrap();
        assert_eq!(
            names(dir.path()),
            [
                "gespan-native.2026-10-24.jsonl",
                "gespan-native.2026-10-25.jsonl"
            ]
        );
        let read = |d| std::fs::read_to_string(dir.path().join(file_name(d))).unwrap();
        assert_eq!(read(day(24)), "{\"a\":1}\n");
        assert_eq!(read(day(25)), "{\"b\":2}\n");
    }

    #[test]
    fn appends_to_existing_file_on_restart() {
        let dir = tempfile::tempdir().unwrap();
        for line in ["one\n", "two\n"] {
            let mut file = DailyFile::new(
                dir.path().into(),
                FakeClock::at(day(25)),
                MAX_DAYS,
                MAX_BYTES,
            )
            .unwrap();
            file.write_all(line.as_bytes()).unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(dir.path().join(file_name(day(25)))).unwrap(),
            "one\ntwo\n"
        );
    }

    #[test]
    fn prune_drops_files_older_than_seven_days() {
        let dir = tempfile::tempdir().unwrap();
        for n in 16..=25 {
            seed(dir.path(), day(n), 10);
        }
        let deleted = prune(dir.path(), day(25), MAX_DAYS, MAX_BYTES).unwrap();
        assert_eq!(deleted.len(), 3);
        let expected: Vec<_> = (19..=25).map(|n| file_name(day(n))).collect();
        assert_eq!(names(dir.path()), expected);
    }

    #[test]
    fn prune_enforces_total_size_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        for n in 22..=25 {
            seed(dir.path(), day(n), 400);
        }
        prune(dir.path(), day(25), MAX_DAYS, 1000).unwrap();
        assert_eq!(names(dir.path()), [file_name(day(24)), file_name(day(25))]);
    }

    #[test]
    fn prune_never_deletes_today() {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path(), day(24), 10);
        seed(dir.path(), day(25), 2000);
        prune(dir.path(), day(25), MAX_DAYS, 1000).unwrap();
        assert_eq!(names(dir.path()), [file_name(day(25))]);
    }

    #[test]
    fn prune_ignores_foreign_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("crashes")).unwrap();
        for name in [
            ".DS_Store",
            "gespan-native.notadate.jsonl",
            "other.2020-01-01.jsonl",
            "gespan-native.2020-01-01.log",
        ] {
            std::fs::write(dir.path().join(name), vec![b'x'; 5000]).unwrap();
        }
        assert!(prune(dir.path(), day(25), MAX_DAYS, 100)
            .unwrap()
            .is_empty());
        assert_eq!(names(dir.path()).len(), 5);
    }

    #[test]
    fn prune_of_a_missing_folder_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            prune(&dir.path().join("absent"), day(25), MAX_DAYS, MAX_BYTES)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn startup_creates_the_folder_and_prunes() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("nested/logs");
        std::fs::create_dir_all(&logs).unwrap();
        seed(&logs, day(1), 10);
        DailyFile::new(logs.clone(), FakeClock::at(day(25)), MAX_DAYS, MAX_BYTES).unwrap();
        assert_eq!(names(&logs), [file_name(day(25))]);
        let fresh = dir.path().join("fresh");
        DailyFile::new(fresh.clone(), FakeClock::at(day(25)), MAX_DAYS, MAX_BYTES).unwrap();
        assert!(fresh.is_dir());
    }

    #[test]
    fn rollover_prunes() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FakeClock::at(day(10));
        let mut file =
            DailyFile::new(dir.path().into(), clock.clone(), MAX_DAYS, MAX_BYTES).unwrap();
        file.write_all(b"old\n").unwrap();
        clock.advance(8);
        file.write_all(b"new\n").unwrap();
        assert_eq!(names(dir.path()), [file_name(day(18))]);
    }

    #[test]
    fn drops_lines_once_today_reaches_budget() {
        let dir = tempfile::tempdir().unwrap();
        let mut file =
            DailyFile::new(dir.path().into(), FakeClock::at(day(25)), MAX_DAYS, 100).unwrap();
        let line = [b'x'; 60];
        for _ in 0..3 {
            assert_eq!(file.write(&line).unwrap(), 60);
        }
        let written = std::fs::metadata(dir.path().join(file_name(day(25))))
            .unwrap()
            .len();
        assert_eq!(written, 60);
    }

    #[test]
    fn a_new_day_restores_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FakeClock::at(day(24));
        let mut file = DailyFile::new(dir.path().into(), clock.clone(), MAX_DAYS, 100).unwrap();
        file.write_all(&[b'x'; 90]).unwrap();
        clock.advance(1);
        file.write_all(&[b'y'; 90]).unwrap();
        // The budget is total: yesterday's file goes to make room for today.
        assert_eq!(names(dir.path()), [file_name(day(25))]);
        assert_eq!(
            std::fs::read(dir.path().join(file_name(day(25)))).unwrap(),
            vec![b'y'; 90]
        );
    }
}
