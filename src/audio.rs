use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub channels: usize,
    pub sample_rate: u32,
    pub bits_per_sample: Option<u32>,
}

struct ProgressTrackedSource {
    file: std::fs::File,
    progress: Arc<AtomicU64>,
}

impl Read for ProgressTrackedSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.file.read(buf)?;
        if n > 0 {
            if let Ok(pos) = self.file.stream_position() {
                self.progress.store(pos, Ordering::Relaxed);
            }
        }
        Ok(n)
    }
}

impl Seek for ProgressTrackedSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let new_pos = self.file.seek(pos)?;
        self.progress.store(new_pos, Ordering::Relaxed);
        Ok(new_pos)
    }
}

impl MediaSource for ProgressTrackedSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        self.file.metadata().ok().map(|m| m.len())
    }
}

/// decode audio file to raw f32 samples
pub fn decode_audio(path: &std::path::Path, progress: Arc<AtomicU64>) -> Result<DecodedAudio, String> {
    let start_read = std::time::Instant::now();
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;

    let src = ProgressTrackedSource {
        file,
        progress,
    };

    let mss = MediaSourceStream::new(Box::new(src), Default::default());

    // hint format registry based on file extension
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }

    let meta_opts = MetadataOptions::default();
    let fmt_opts = FormatOptions::default();

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)
        .map_err(|e| format!("unsupported format: {}", e))?;

    let mut format = probed.format;

    // get first track with known codec
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| "no supported audio track found".to_string())?;

    let dec_opts = DecoderOptions::default();

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &dec_opts)
        .map_err(|e| format!("unsupported codec: {}", e))?;

    let track_id = track.id;
    let channels = track.codec_params.channels.ok_or("No channels metadata")?.count();
    let sample_rate = track.codec_params.sample_rate.ok_or("No sample rate metadata")?;
    let bits_per_sample = track.codec_params.bits_per_sample;

    let mut all_samples = Vec::new();

    // decode all packets
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(ref err)) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.to_string()),
        };

        while !format.metadata().is_latest() {
            format.metadata().pop();
        }

        if packet.track_id() == track_id {
            match decoder.decode(&packet) {
                Ok(decoded) => {
                    let mut sample_buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
                    sample_buffer.copy_interleaved_ref(decoded);
                    all_samples.extend_from_slice(sample_buffer.samples());
                }
                Err(Error::IoError(_) | Error::DecodeError(_)) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    if all_samples.is_empty() {
        return Err("Could not parse any valid audio samples from the file.".into());
    }
    println!("Reading and decoding file took: {:?}", start_read.elapsed());

    Ok(DecodedAudio {
        samples: all_samples,
        channels,
        sample_rate,
        bits_per_sample,
    })
}
