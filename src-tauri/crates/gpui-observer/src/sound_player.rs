//! Native playback of the same notification assets the Vue workflow uses.

use chatspeed_gpui_observer::chimes::Chime;
use rodio::{Decoder, OutputStreamBuilder, Sink};
use std::{io::Cursor, sync::mpsc};

pub struct SoundPlayer {
    commands: mpsc::SyncSender<Chime>,
    failures: mpsc::Receiver<()>,
}

impl SoundPlayer {
    pub fn start() -> std::io::Result<Self> {
        let (commands, receive) = mpsc::sync_channel(4);
        let (report, failures) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("gpui-workflow-audio".into())
            .spawn(move || {
                let mut output = None;
                let mut sinks: [Option<Sink>; 2] = [None, None];
                while let Ok(chime) = receive.recv() {
                    let (index, bytes) = sound(chime);
                    if output.is_none() {
                        output = OutputStreamBuilder::open_default_stream().ok();
                    }
                    let Some(stream) = output.as_ref() else {
                        let _ = report.try_send(());
                        continue;
                    };
                    let Ok(source) = Decoder::try_from(Cursor::new(bytes)) else {
                        let _ = report.try_send(());
                        continue;
                    };
                    // Match Vue: restart only the chime of the same kind.
                    let sink = Sink::connect_new(stream.mixer());
                    sink.append(source);
                    sinks[index] = Some(sink);
                }
            })?;
        Ok(Self { commands, failures })
    }

    pub fn play(&self, chime: Chime) {
        let _ = self.commands.try_send(chime);
    }

    pub fn playback_failed(&self) -> bool {
        self.failures.try_recv().is_ok()
    }
}

fn sound(chime: Chime) -> (usize, &'static [u8]) {
    match chime {
        Chime::Approval => (
            0,
            include_bytes!("../../../../public/sound/notification.mp3"),
        ),
        Chime::Completion => (1, include_bytes!("../../../../public/sound/success.mp3")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_bundled_vue_sounds_decode() {
        for chime in Chime::ALL {
            let (_, bytes) = sound(chime);
            let mut samples = Decoder::try_from(Cursor::new(bytes)).expect("valid MP3");
            assert!(samples.any(|sample| sample.abs() > 0.001));
        }
    }
}
