//! Playing on iOS as the system expects an audio app to: an audio
//! session, the lock screen's Now Playing, and the remote commands.
//!
//! - **The session** — play-and-record (the engine takes the microphone
//!   too), out of the loudspeaker rather than the earpiece, and allowed to
//!   Bluetooth (A2DP) headphones and speakers and AirPlay. Without it iOS
//!   gave a play-and-record app the earpiece and refused Bluetooth. Set as
//!   the app starts, before the engine opens the device.
//! - **Now Playing** — the song, where it is and how long it is, and whether
//!   it plays: the lock screen, Control Center, a car or headphones show it.
//! - **Remote commands** — play, pause, next and previous song from those
//!   same places. A command is queued here ([`take_commands`]) and the
//!   shell carries it out, where the transport and the set are.

use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_avf_audio::{
    AVAudioSession, AVAudioSessionCategoryOptions, AVAudioSessionCategoryPlayAndRecord,
    AVAudioSessionModeDefault,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString};
use objc2_media_player::{
    MPMediaItemPropertyAlbumTitle, MPMediaItemPropertyPlaybackDuration, MPMediaItemPropertyTitle,
    MPNowPlayingInfoCenter, MPNowPlayingInfoPropertyElapsedPlaybackTime,
    MPNowPlayingInfoPropertyPlaybackRate, MPRemoteCommand, MPRemoteCommandCenter,
    MPRemoteCommandEvent, MPRemoteCommandHandlerStatus,
};

/// Set the audio session up for playing a set: play-and-record, the
/// loudspeaker, Bluetooth and AirPlay allowed, and active.
pub fn configure_session() {
    // SAFETY: AVAudioSession is thread-safe; the statics are Apple's
    // constants, read after the framework has loaded.
    unsafe {
        let session = AVAudioSession::sharedInstance();
        let (Some(category), Some(mode)) = (
            AVAudioSessionCategoryPlayAndRecord,
            AVAudioSessionModeDefault,
        ) else {
            return;
        };
        let options = AVAudioSessionCategoryOptions::DefaultToSpeaker
            | AVAudioSessionCategoryOptions::AllowBluetoothA2DP
            | AVAudioSessionCategoryOptions::AllowAirPlay;
        if let Err(e) = session.setCategory_mode_options_error(category, mode, options) {
            tracing::warn!(error = ?e, "ios audio: the session's category was refused");
        }
        if let Err(e) = session.setActive_error(true) {
            tracing::warn!(error = ?e, "ios audio: the session did not become active");
        }
    }
}

/// A remote command, as the shell carries it out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
}

static COMMANDS: Mutex<Vec<Command>> = Mutex::new(Vec::new());

/// The remote commands since the last call, oldest first.
pub fn take_commands() -> Vec<Command> {
    COMMANDS
        .lock()
        .map(|mut queue| std::mem::take(&mut *queue))
        .unwrap_or_default()
}

/// Listen for the remote commands: play, pause, play/pause, next and
/// previous. Once, on the main thread.
pub fn install_commands() {
    fn listen(command: &MPRemoteCommand, what: Command) {
        let handler = block2::RcBlock::new(
            move |_event: std::ptr::NonNull<MPRemoteCommandEvent>| -> MPRemoteCommandHandlerStatus {
                if let Ok(mut queue) = COMMANDS.lock() {
                    queue.push(what);
                }
                MPRemoteCommandHandlerStatus::Success
            },
        );
        // SAFETY: the command centre is used from the main thread; the
        // block is retained by the command for as long as it listens.
        unsafe {
            command.setEnabled(true);
            let _target = command.addTargetWithHandler(&handler);
        }
    }
    // SAFETY: as above.
    let center = unsafe { MPRemoteCommandCenter::sharedCommandCenter() };
    unsafe {
        listen(&center.playCommand(), Command::Play);
        listen(&center.pauseCommand(), Command::Pause);
        listen(&center.togglePlayPauseCommand(), Command::Toggle);
        listen(&center.nextTrackCommand(), Command::Next);
        listen(&center.previousTrackCommand(), Command::Previous);
    }
}

/// What Now Playing shows.
#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    /// The set it is in, shown as its album.
    pub set: String,
    /// Seconds into the song, and its length.
    pub elapsed: f64,
    pub duration: f64,
    pub playing: bool,
}

/// Show `now` as Now Playing (the lock screen, Control Center, a car).
/// iOS moves the position on by itself while the rate is 1, so this is
/// needed on a change, not every frame.
pub fn show(now: &NowPlaying) {
    fn object(o: Retained<impl objc2::Message>) -> Retained<AnyObject> {
        // SAFETY: every Objective-C object is an AnyObject.
        unsafe { Retained::cast_unchecked(o) }
    }
    // SAFETY: Apple's key constants, and an info dictionary of the types
    // MPNowPlayingInfoCenter documents for each.
    unsafe {
        let keys: [&NSString; 5] = [
            MPMediaItemPropertyTitle,
            MPMediaItemPropertyAlbumTitle,
            MPNowPlayingInfoPropertyElapsedPlaybackTime,
            MPMediaItemPropertyPlaybackDuration,
            MPNowPlayingInfoPropertyPlaybackRate,
        ];
        let values: [Retained<AnyObject>; 5] = [
            object(NSString::from_str(&now.title)),
            object(NSString::from_str(&now.set)),
            object(NSNumber::new_f64(now.elapsed.max(0.0))),
            object(NSNumber::new_f64(now.duration.max(0.0))),
            object(NSNumber::new_f64(if now.playing { 1.0 } else { 0.0 })),
        ];
        let info = NSDictionary::from_retained_objects(&keys, &values);
        MPNowPlayingInfoCenter::defaultCenter().setNowPlayingInfo(Some(&info));
    }
}

/// Nothing playing: Now Playing cleared (back at the start screen).
pub fn clear() {
    // SAFETY: the default centre, its info set to none.
    unsafe {
        MPNowPlayingInfoCenter::defaultCenter().setNowPlayingInfo(None);
    }
}
