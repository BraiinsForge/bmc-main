// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use std::os::fd::{AsFd, AsRawFd, RawFd};

use anyhow::bail;
use nix::errno::Errno;
use nix::libc::time_t;
use nix::sys::time::TimeSpec;
use nix::sys::timerfd::{ClockId, Expiration, TimerFd, TimerFlags, TimerSetTimeFlags};
use nix::unistd::read;
use tokio::io::unix::AsyncFd;
use tokio::sync::watch;
use tracing::{debug, error};

pub(super) fn watch() -> std::io::Result<watch::Receiver<u64>> {
    let timer = AsyncFd::new(StepTimer::new()?)?;
    let (sender, receiver) = watch::channel(0);
    tokio::spawn(follow(timer, sender));
    Ok(receiver)
}

/// An absolute `CLOCK_REALTIME` timer that the kernel cancels on every clock step.
struct StepTimer(TimerFd);

impl AsRawFd for StepTimer {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_fd().as_raw_fd()
    }
}

impl StepTimer {
    fn new() -> nix::Result<Self> {
        Self::expiring_at(TimeSpec::new(time_t::MAX, 0))
    }

    fn expiring_at(expiry: TimeSpec) -> nix::Result<Self> {
        let timer = TimerFd::new(
            ClockId::CLOCK_REALTIME,
            TimerFlags::TFD_NONBLOCK | TimerFlags::TFD_CLOEXEC,
        )?;
        let this = Self(timer);
        this.arm(expiry)?;
        Ok(this)
    }

    fn arm(&self, expiry: TimeSpec) -> nix::Result<()> {
        let armed = self.0.set(
            Expiration::OneShot(expiry),
            TimerSetTimeFlags::TFD_TIMER_ABSTIME | TimerSetTimeFlags::TFD_TIMER_CANCEL_ON_SET,
        );
        match armed {
            // The kernel arms the timer before it reports a step since the last read.
            // That step came during startup or while another was being reported,
            // so it needs no report of its own.
            Err(Errno::ECANCELED) => Ok(()),
            armed => armed,
        }
    }

    /// Returns whether the clock stepped since the last call, re-arming if it did.
    fn take_step(&self) -> anyhow::Result<bool> {
        let mut expirations = [0_u8; 8];
        loop {
            match read(self.0.as_fd(), &mut expirations) {
                Err(Errno::EINTR) => {}
                Err(Errno::ECANCELED) => {
                    self.arm(TimeSpec::new(time_t::MAX, 0))?;
                    return Ok(true);
                }
                Err(Errno::EAGAIN) => return Ok(false),
                Err(err) => return Err(err.into()),
                // On a 32-bit `time_t` the never-expiring timer expires on 2038-01-19.
                Ok(_) => bail!("the clock passed the step timer's expiry"),
            }
        }
    }
}

async fn follow(timer: AsyncFd<StepTimer>, sender: watch::Sender<u64>) {
    if let Err(err) = count_steps(&timer, &sender).await {
        error!(error = %err, "Wall-clock step timer failed, steps go unnoticed");
    }
}

async fn count_steps(
    timer: &AsyncFd<StepTimer>,
    sender: &watch::Sender<u64>,
) -> anyhow::Result<()> {
    loop {
        let mut ready = timer.readable().await?;
        if ready.get_inner().take_step()? {
            debug!("Wall clock stepped");
            sender.send_modify(|steps| *steps = steps.wrapping_add(1));
        } else {
            ready.clear_ready();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn step_timer_stays_quiet_without_a_step() {
        let timer = StepTimer::new().expect("BUG: arming a CLOCK_REALTIME timerfd must succeed");

        let stepped = timer
            .take_step()
            .expect("BUG: reading a quiet timerfd must succeed");

        assert!(
            !stepped,
            "nothing stepped the clock, so there is nothing to read"
        );
    }

    #[test]
    fn step_timer_fails_on_expiry_instead_of_reporting_a_step() {
        let timer = StepTimer::expiring_at(TimeSpec::new(1, 0))
            .expect("BUG: arming a CLOCK_REALTIME timerfd must succeed");
        let deadline = Instant::now() + Duration::from_secs(1);

        let reading = loop {
            match timer.take_step() {
                Ok(false) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                reading => break reading,
            }
        };

        assert!(
            reading.is_err(),
            "a timer armed in the past expires, which must not read as a clock step: {reading:?}"
        );
    }

    #[tokio::test]
    async fn watch_registers_the_timer_with_the_runtime() {
        watch().expect("BUG: a CLOCK_REALTIME timerfd must register with the Tokio reactor");
    }
}
