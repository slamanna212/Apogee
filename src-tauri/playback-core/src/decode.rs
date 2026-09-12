//! Common decoder boundary: demuxed access units, never container bytes.
use symphonia::core::{
    codecs::audio::{AudioCodecParameters, AudioDecoder, AudioDecoderOptions, well_known},
    packet::PacketRef,
};
use transmux::{CodecConfig, Sample, TrackSpec};

/// A worker-owned block. Never construct or allocate these in an output callback.
#[derive(Debug)]
pub struct PcmBlock {
    pub rate: u32,
    pub channels: usize,
    pub pts: i64,
    pub timescale: u32,
    pub samples: Vec<f32>,
}

pub struct AudioTrackDecoder {
    track_id: u32,
    timescale: u32,
    decoder: Box<dyn AudioDecoder>,
}

impl AudioTrackDecoder {
    /// Whether this track is an audio track this app can decode.
    ///
    /// Used to select the audio track deterministically and to skip video or
    /// otherwise unsupported tracks without attempting to build a decoder.
    pub fn supports(spec: &TrackSpec) -> bool {
        match &spec.config {
            CodecConfig::Aac { .. } => true,
            CodecConfig::MpegAudio { layer, .. } => layer.number() == 3,
            _ => false,
        }
    }

    pub fn new(spec: &TrackSpec) -> Result<Self, String> {
        if spec.timescale == 0 {
            return Err("audio track has zero timescale".into());
        }
        let mut params = AudioCodecParameters::new();
        match &spec.config {
            CodecConfig::Aac {
                esds, sample_rate, ..
            } => {
                let asc = esds
                    .es_descriptor
                    .decoder_config
                    .as_ref()
                    .and_then(|config| config.decoder_specific_info.as_ref())
                    .ok_or("AAC track is missing AudioSpecificConfig")?;
                params
                    .for_codec(well_known::CODEC_ID_AAC)
                    .with_sample_rate(*sample_rate)
                    .with_extra_data(asc.data.clone().into_boxed_slice());
            }
            CodecConfig::MpegAudio {
                layer, sample_rate, ..
            } if layer.number() == 3 => {
                params
                    .for_codec(well_known::CODEC_ID_MP3)
                    .with_sample_rate(*sample_rate);
            }
            _ => return Err("audio track codec is outside MP3/AAC scope".into()),
        }
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(|e| format!("could not initialize audio decoder: {e}"))?;
        Ok(Self {
            track_id: spec.track_id,
            timescale: spec.timescale,
            decoder,
        })
    }

    pub fn track_id(&self) -> u32 {
        self.track_id
    }

    pub fn reset(&mut self) {
        self.decoder.reset();
    }

    pub fn decode(&mut self, sample: &Sample) -> Result<PcmBlock, String> {
        let pts = sample
            .pts
            .ok_or("audio sample is missing presentation time")?;
        let duration = sample.duration.ok_or("audio sample is missing duration")?;
        let packet = PacketRef::new(self.track_id, pts.into(), duration.into(), &sample.data);
        let audio = self
            .decoder
            .decode_ref(&packet)
            .map_err(|e| format!("could not decode audio access unit: {e}"))?;
        let mut samples = Vec::with_capacity(audio.samples_interleaved());
        audio.copy_to_vec_interleaved(&mut samples);
        Ok(PcmBlock {
            rate: audio.spec().rate(),
            channels: audio.spec().channels().count(),
            pts,
            timescale: self.timescale,
            samples,
        })
    }
}
