// Copyright © 2018 Cormac O'Brien
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

mod music;
use bevy::{ecs::system::Resource, render::extract_resource::ExtractResource};
pub use music::MusicPlayer;

use std::{
    cell::{Cell, RefCell},
    io::{self, BufReader, Cursor, Read},
    iter,
    sync::Arc,
};

use crate::common::vfs::{Vfs, VfsError};

use cgmath::{InnerSpace, Vector3};
use chrono::Duration;
use rodio::{
    source::{Buffered, SamplesConverter},
    Decoder, OutputStreamHandle, Sink, Source,
};
use thiserror::Error;

pub const DISTANCE_ATTENUATION_FACTOR: f32 = 0.001;
const MAX_ENTITY_CHANNELS: usize = 128;

#[derive(Error, Debug)]
pub enum SoundError {
    #[error("No such music track: {0}")]
    NoSuchTrack(String),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Virtual filesystem error: {0}")]
    Vfs(#[from] VfsError),
    #[error("WAV decoder error: {0}")]
    Decoder(#[from] rodio::decoder::DecoderError),
}

/// Data needed for sound spatialization.
///
/// This struct is updated every frame.
#[derive(Debug, Clone)]
pub struct Listener {
    origin: Vector3<f32>,
    left_ear: Vector3<f32>,
    right_ear: Vector3<f32>,
}

impl Listener {
    pub fn new() -> Listener {
        Listener {
            origin: Vector3::new(0.0, 0.0, 0.0),
            left_ear: Vector3::new(0.0, 0.0, 0.0),
            right_ear: Vector3::new(0.0, 0.0, 0.0),
        }
    }

    pub fn origin(&self) -> Vector3<f32> {
        self.origin
    }

    pub fn left_ear(&self) -> Vector3<f32> {
        self.left_ear
    }

    pub fn right_ear(&self) -> Vector3<f32> {
        self.right_ear
    }

    pub fn set_origin(&mut self, new_origin: Vector3<f32>) {
        self.origin = new_origin;
    }

    pub fn set_left_ear(&mut self, new_origin: Vector3<f32>) {
        self.left_ear = new_origin;
    }

    pub fn set_right_ear(&mut self, new_origin: Vector3<f32>) {
        self.right_ear = new_origin;
    }

    pub fn attenuate(
        &self,
        emitter_origin: Vector3<f32>,
        base_volume: f32,
        attenuation: f32,
    ) -> f32 {
        let decay =
            (emitter_origin - self.origin).magnitude() * attenuation * DISTANCE_ATTENUATION_FACTOR;
        let volume = ((1.0 - decay) * base_volume).max(0.0);
        volume
    }
}

type SourceInner = Buffered<SamplesConverter<Decoder<Cursor<Vec<u8>>>, f32>>;
#[derive(Clone)]
pub struct AudioSource(Arc<SourceInner>);

impl Iterator for AudioSource {
    type Item = <SourceInner as Iterator>::Item;

    fn next(&mut self) -> Option<Self::Item> {
        Arc::make_mut(&mut self.0).next()
    }
}

impl Source for AudioSource {
    fn current_frame_len(&self) -> Option<usize> {
        self.0.current_frame_len()
    }

    fn channels(&self) -> u16 {
        self.0.channels()
    }

    fn sample_rate(&self) -> u32 {
        self.0.sample_rate()
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        self.0.total_duration()
    }
}

impl AudioSource {
    pub fn load<S>(vfs: &Vfs, name: S) -> Result<AudioSource, SoundError>
    where
        S: AsRef<str>,
    {
        let name = name.as_ref();
        let full_path = "sound/".to_owned() + name;
        let mut file = vfs.open(&full_path)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;

        let src = Decoder::new(Cursor::new(data))?
            .convert_samples()
            .buffered();

        Ok(AudioSource(src.into()))
    }
}

pub struct StaticSound {
    origin: Vector3<f32>,
    sink: Sink,
    volume: f32,
    attenuation: f32,
}

impl StaticSound {
    pub fn new(
        stream: OutputStreamHandle,
        origin: Vector3<f32>,
        src: AudioSource,
        volume: f32,
        attenuation: f32,
        listener: &Listener,
    ) -> StaticSound {
        // TODO: handle PlayError once PR accepted
        let sink = Sink::try_new(&stream).unwrap();
        let infinite = src.repeat_infinite();
        sink.append(infinite);
        sink.set_volume(listener.attenuate(origin, volume, attenuation));

        StaticSound {
            origin,
            sink,
            volume,
            attenuation,
        }
    }

    pub fn update(&mut self, listener: &Listener) {
        self.sink
            .set_volume(listener.attenuate(self.origin, self.volume, self.attenuation));
    }
}

/// Represents a single audio channel, capable of playing one sound at a time.
pub struct Channel {
    sink: Option<Sink>,
    master_vol: f32,
    attenuation: f32,
}

impl Channel {
    /// Create a new `Channel` backed by the given `Device`.
    pub fn new() -> Channel {
        Channel {
            sink: None,
            master_vol: 0.0,
            attenuation: 0.0,
        }
    }

    /// Play a new sound on this channel, cutting off any sound that was previously playing.
    pub fn play(
        &mut self,
        src: AudioSource,
        ent_pos: Vector3<f32>,
        listener: &Listener,
        volume: f32,
        attenuation: f32,
        stream: OutputStreamHandle,
    ) {
        self.master_vol = volume;
        self.attenuation = attenuation;

        // stop the old sound
        self.sink = None;

        // start the new sound
        let new_sink = Sink::try_new(&stream).unwrap();
        new_sink.append(src);
        new_sink.set_volume(listener.attenuate(ent_pos, self.master_vol, self.attenuation));

        self.sink = Some(new_sink);
    }

    pub fn update(&mut self, ent_pos: Vector3<f32>, listener: &Listener) {
        let replace_sink = if let Some(sink) = &mut self.sink {
            // attenuate using quake coordinates since distance is the same either way
            sink.set_volume(listener.attenuate(ent_pos, self.master_vol, self.attenuation));
            sink.empty()
        } else {
            false
        };

        // if the sink isn't in use, free it
        if replace_sink {
            self.sink = None;
        }
    }

    /// Stop the sound currently playing on this channel, if there is one.
    pub fn stop(&mut self) {
        self.sink = None;
    }

    /// Returns whether or not this `Channel` is currently in use.
    pub fn in_use(&self) -> bool {
        match &self.sink {
            Some(sink) => sink.empty(),
            None => false,
        }
    }
}

#[derive(Resource, Clone, ExtractResource)]
pub struct AudioOut(pub OutputStreamHandle);

#[derive(Clone)]
pub struct EntityChannel {
    start_time: Duration,
    // if None, sound is associated with a temp entity
    ent_id: Option<usize>,
    ent_channel: i8,
    channel: Arc<Channel>,
}

impl EntityChannel {
    pub fn channel(&self) -> &Channel {
        &self.channel
    }

    pub fn channel_mut(&mut self) -> &mut Channel {
        // TODO: We only wrap this in an arc so we can send render state to the render thread
        //       for pipelined rendering - the audio data should never actually be shared
        Arc::get_mut(&mut self.channel).expect("This should never actually be shared!")
    }

    pub fn entity_id(&self) -> Option<usize> {
        self.ent_id
    }

    pub fn channel_id(&self) -> i8 {
        self.ent_channel
    }
}

#[derive(Clone)]
pub struct EntityMixer {
    channels: im::Vector<Option<EntityChannel>>,
    // TODO: This should always be passed down from `World`, but right now everything is too
    //       monolithic to make that possible
    output_stream: OutputStreamHandle,
}

impl EntityMixer {
    pub fn new(output_stream: OutputStreamHandle) -> EntityMixer {
        let channels = iter::repeat_n(None, MAX_ENTITY_CHANNELS).collect();

        EntityMixer {
            channels,
            output_stream,
        }
    }

    fn find_free_channel(&self, ent_id: Option<usize>, ent_channel: i8) -> usize {
        let mut oldest = 0;

        for (i, channel) in self.channels.iter().enumerate() {
            match *channel {
                Some(ref chan) => {
                    // if this channel is free, return it
                    if !chan.channel.in_use() {
                        return i;
                    }

                    // replace sounds on the same entity channel
                    if ent_channel != 0
                        && chan.ent_id == ent_id
                        && (chan.ent_channel == ent_channel || ent_channel == -1)
                    {
                        return i;
                    }

                    // TODO: don't clobber player sounds with monster sounds

                    // keep track of which sound started the earliest
                    match self.channels[oldest] {
                        Some(ref o) => {
                            if chan.start_time < o.start_time {
                                oldest = i;
                            }
                        }
                        None => oldest = i,
                    }
                }

                None => return i,
            }
        }

        // if there are no good channels, just replace the one that's been running the longest
        oldest
    }

    pub fn start_sound(
        &mut self,
        src: AudioSource,
        time: Duration,
        ent_id: Option<usize>,
        ent_channel: i8,
        volume: f32,
        attenuation: f32,
        origin: Vector3<f32>,
        listener: &Listener,
    ) {
        let chan_id = self.find_free_channel(ent_id, ent_channel);
        let mut new_channel = Channel::new();

        new_channel.play(
            src,
            origin,
            listener,
            volume,
            attenuation,
            self.output_stream.clone(),
        );
        self.channels[chan_id] = Some(EntityChannel {
            start_time: time,
            ent_id,
            ent_channel,
            channel: new_channel.into(),
        })
    }

    pub fn stop_sound(&mut self, ent_id: Option<usize>, ent_channel: i8) {
        let matching_channels = self
            .channels
            .iter_mut()
            .filter_map(|chan| chan.as_mut())
            .filter(|c| c.entity_id() == ent_id && c.channel_id() == ent_channel);
        for c in matching_channels {
            c.channel_mut().stop()
        }
    }

    pub fn iter_entity_channels(&self) -> impl Iterator<Item = &EntityChannel> {
        self.channels.iter().filter_map(|e| e.as_ref())
    }

    pub fn iter_entity_channels_mut(&mut self) -> impl Iterator<Item = &mut EntityChannel> {
        self.channels.iter_mut().filter_map(|e| e.as_mut())
    }
}
