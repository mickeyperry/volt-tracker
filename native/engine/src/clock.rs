//! The transport — `pump()` from beta2.html, as a piece of arithmetic you can test.
//!
//! The browser's `pump()` runs on a 30 ms `setInterval` and schedules every row that falls inside
//! the next 160 ms, handing each one an absolute AudioContext time. Nothing is ever played "now":
//! the timer only decides *what to book*, and the audio clock decides when it sounds. That is why
//! VOLT's timing survives a stuttering main thread, and it is the property worth preserving above
//! all others in the port.
//!
//! Deliberately free of any audio types. The clock takes a time in seconds and returns which rows
//! are due — so the whole of it can be tested with plain numbers, at a thousand times real speed,
//! with no device open and no listening involved. The bugs that matter here (a row scheduled
//! twice, a row skipped at a loop boundary, drift accumulating over a long song) are exactly the
//! ones that are miserable to hear and trivial to assert.

/// A row that has come due, with the absolute time it must sound at.
#[derive(Debug, PartialEq)]
pub struct Due {
    pub row: usize,
    pub t: f64,
}

pub struct Clock {
    pub bpm: f64,
    /// Rows per beat. Editable in VOLT and rescales every pattern, so it is not a constant.
    pub lpb: f64,
    /// Pattern length in rows. Playback wraps at this.
    pub rows: usize,
    /// The next row to be scheduled, and the exact time it belongs at. Time is carried forward by
    /// repeated addition of `row_dur()` from the start, never recomputed from a wall clock —
    /// that is what stops drift accumulating over a long song.
    next_row: usize,
    next_t: f64,
    playing: bool,
}

impl Clock {
    pub fn new(bpm: f64, lpb: f64, rows: usize) -> Self {
        Self { bpm, lpb, rows: rows.max(1), next_row: 0, next_t: 0.0, playing: false }
    }

    /// `rowDur() = 60/bpm/lpb`.
    pub fn row_dur(&self) -> f64 {
        60.0 / self.bpm / self.lpb
    }

    /// Begin at `t`, which should be a little ahead of the context's current time so the first row
    /// is booked rather than missed.
    pub fn start(&mut self, t: f64, from_row: usize) {
        self.next_row = from_row % self.rows;
        self.next_t = t;
        self.playing = true;
    }

    pub fn stop(&mut self) {
        self.playing = false;
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    /// Every row whose time falls within `lookahead` seconds of `now`.
    ///
    /// Call it far more often than the lookahead is long — the browser uses a 30 ms tick against
    /// 160 ms of lookahead, five times the margin — so that a late tick still books its rows
    /// before they were due. A tick that arrives late returns more rows rather than losing any,
    /// which is the behaviour that makes the transport robust to a busy UI thread.
    pub fn due(&mut self, now: f64, lookahead: f64) -> Vec<Due> {
        let mut out = Vec::new();
        if !self.playing {
            return out;
        }
        let dur = self.row_dur();
        let horizon = now + lookahead;
        // Bounded so a pathological bpm (or a lookahead someone set to an hour) cannot spin here
        // forever holding up the caller's tick.
        while self.next_t < horizon && out.len() < 4096 {
            out.push(Due { row: self.next_row, t: self.next_t });
            self.next_row = (self.next_row + 1) % self.rows;
            self.next_t += dur;
        }
        out
    }

    /// `Txx` — the sub-row delay that gives VOLT triplets and the arp's off-grid notes. The value
    /// is in 192nds of a row, so `T60` is a third of the way to the next row.
    pub fn t_offset(&self, xx: u8) -> f64 {
        self.row_dur() * (xx as f64 / 192.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_duration_matches_the_browser() {
        // 120 bpm, 4 rows/beat -> 125 ms, the figure beta2 shows in the transport.
        let c = Clock::new(120.0, 4.0, 64);
        assert!((c.row_dur() - 0.125).abs() < 1e-12);
        // lpb is editable; 6 rows/beat is the usual triplet feel.
        let c = Clock::new(120.0, 6.0, 64);
        assert!((c.row_dur() - 0.083333333333).abs() < 1e-9);
    }

    #[test]
    fn every_row_is_scheduled_exactly_once() {
        // The bug this guards against is the one you cannot un-hear: a doubled or dropped row.
        let mut c = Clock::new(120.0, 4.0, 16);
        c.start(1.0, 0);
        let mut seen = Vec::new();
        let mut now = 1.0;
        // Two full patterns' worth of 30 ms ticks with 160 ms lookahead, as the browser runs it.
        while now < 1.0 + 32.0 * c.row_dur() {
            for d in c.due(now, 0.160) {
                seen.push(d);
            }
            now += 0.030;
        }
        assert!(seen.len() >= 32, "only {} rows scheduled", seen.len());
        for (i, d) in seen.iter().take(32).enumerate() {
            assert_eq!(d.row, i % 16, "row {i} out of order");
            let want = 1.0 + i as f64 * 0.125;
            assert!((d.t - want).abs() < 1e-9, "row {i} at {} not {want}", d.t);
        }
    }

    #[test]
    fn a_late_tick_loses_nothing() {
        // The whole reason for scheduling ahead: the UI thread stalls, the tick arrives 200 ms
        // late, and the rows in between must still be booked — not skipped.
        let mut c = Clock::new(120.0, 4.0, 16);
        c.start(0.0, 0);
        let first = c.due(0.0, 0.160);
        assert_eq!(first.len(), 2, "0 and 0.125 fall inside 160 ms");
        let after_stall = c.due(0.5, 0.160);
        assert_eq!(after_stall.first().unwrap().row, 2, "resumed at the wrong row");
        // Times stay on the original grid rather than being pulled toward `now`.
        assert!((after_stall[0].t - 0.250).abs() < 1e-9);
    }

    #[test]
    fn time_does_not_drift_over_a_long_song() {
        // Ten minutes at 175 bpm. Accumulating error here would be inaudible for a bar and
        // obviously wrong by the end, which is the worst kind of bug to find late.
        let mut c = Clock::new(175.0, 4.0, 64);
        c.start(0.0, 0);
        let mut last = 0.0;
        let mut n = 0usize;
        let mut now = 0.0;
        while now < 600.0 {
            for d in c.due(now, 0.160) {
                last = d.t;
                n += 1;
            }
            now += 0.030;
        }
        let want = (n - 1) as f64 * c.row_dur();
        assert!((last - want).abs() < 1e-6, "drifted by {} s over {n} rows", (last - want).abs());
    }

    #[test]
    fn stopped_clocks_schedule_nothing() {
        let mut c = Clock::new(120.0, 4.0, 16);
        assert!(c.due(0.0, 1.0).is_empty(), "scheduled before start");
        c.start(0.0, 0);
        assert!(!c.due(0.0, 0.160).is_empty());
        c.stop();
        assert!(c.due(1.0, 1.0).is_empty(), "kept scheduling after stop");
    }

    #[test]
    fn txx_is_192nds_of_a_row() {
        let c = Clock::new(120.0, 4.0, 16);
        assert!((c.t_offset(0) - 0.0).abs() < 1e-12);
        // T60 = 60/192 of a 125 ms row — how a triplet lands between rows.
        assert!((c.t_offset(64) - 0.125 / 3.0).abs() < 1e-9);
        assert!((c.t_offset(96) - 0.0625).abs() < 1e-12);
    }
}
