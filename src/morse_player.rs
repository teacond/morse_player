// morse_player.rs
// Copyright (C) 2025-2026  Jaŭhien Lavonćjeŭ <jauhien.lavoncjeu@gmail.com>
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

use std::{cell::{Cell, RefCell}, collections::HashMap, num::NonZero, rc::Rc, sync::{Arc, Mutex, LazyLock}, time::Duration};
use rodio::{MixerDeviceSink, DeviceSinkBuilder, Player};
use strum_macros::{Display, EnumString};
use tokio::runtime::Runtime;
use std::f32::consts::PI;
use tokio_util::sync::CancellationToken;
use derive_more::Debug;

pub static MORSE_CODE: LazyLock<HashMap<String, HashMap<char, String>>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("morse.json")).unwrap()
});

const LETTERS_DURATION: f64 = 0.05;
const DIGITS_DURATION: f64 = 0.034;
const MIXED_DURATION: f64 = 0.042;
const FADE_IN: f32 = 0.0002;
const FADE_OUT: f32 = 0.0002;
const SINK_BUFFER_SIZE: u32 = 3;

#[derive(PartialEq, Default, Clone, Copy)]
pub enum TextType {
    #[default]
    Letters,
    Digits,
    Mixed,
}

#[derive(PartialEq, Default, Clone, Copy)]
pub enum WaveType {
    #[default]
    Square,
    Triangle,
    Sawtooth,
    Sine
}

#[derive(PartialEq, Display, Default, Clone, Copy, EnumString)]
#[strum(serialize_all = "kebab_case")]
pub enum Alphabet {
    #[default]
    Latin,
    Cyrillic,
    Greek,
    Hebrew,
    Arabic,
    Persian,
    Korean
}

#[derive(Clone)]
struct WaveGenerator {
    wave_type: WaveType,
    phase: Cell<f32>,
    phase_inc: f32,
    max_k: usize
}

impl WaveGenerator {
    pub fn new(wave_type: WaveType, frequency: f32, sample_rate: u32) -> WaveGenerator {
        WaveGenerator {
            wave_type: wave_type,
            phase: Cell::new(0.0),
            phase_inc: 2.0 * PI * frequency / sample_rate as f32,
            max_k: (sample_rate as f32 / (2.0 * frequency)).floor() as usize,
        }
    }

    #[inline]
    pub fn next(&self) -> f32 {
        let mut sample = 0.0;
        match self.wave_type {
            WaveType::Square => {
                for k in 1..=self.max_k {
                    let n = (2 * k - 1) as f32;
                    sample += (self.phase.get() * n).sin() / n;
                }
                sample *= 4.0 / PI;
            }
            WaveType::Triangle => {
                let mut sign: f32 = 1.0;
                for k in 0..=self.max_k-1 {
                    let n = 2 * k + 1;
                    sample += sign * ((self.phase.get() * n as f32).sin() / n.pow(2) as f32);
                    sign = -sign;
                }
                sample *= 8.0 / PI.powi(2);
            }
            WaveType::Sawtooth => {
                let mut sign: f32 = -1.0;
                for k in 1..=self.max_k {
                    sample += sign * ((self.phase.get() * k as f32).sin() / k as f32);
                    sign = -sign;
                }
                sample *= -2.0 / PI;
            }
            WaveType::Sine => {
                sample = self.phase.get().sin();
            }
        };

        self.skip_samples(1);

        sample
    }

    #[inline]
    fn skip_samples(&self, n: usize) {
        self.phase.set((self.phase.get() + n as f32 * self.phase_inc) % (2.0 * PI));
    }
}


#[derive(Clone, Debug)]
pub struct MorsePlayer {
    #[debug(skip)]
    _stream: Rc<MixerDeviceSink>,
    #[debug(skip)]
    player: Arc<Mutex<Player>>,
    cancellation_token: Rc<RefCell<CancellationToken>>,
    actions: Rc<RefCell<HashMap<char, (u8, u32)>>>,
    alphabet: Rc<RefCell<HashMap<char, String>>>
}

impl MorsePlayer {
    #[inline]
    pub fn new() -> MorsePlayer {
        let mut stream = DeviceSinkBuilder::open_default_sink().unwrap();
        let sink = Player::connect_new(stream.mixer());
        stream.log_on_drop(false);
        sink.set_volume(0.5);
        let mut morse_delays = HashMap::new();
        morse_delays.insert('.', (0, 1));
        morse_delays.insert('-', (0, 3));
        morse_delays.insert('*', (1, 1));
        morse_delays.insert('$', (1, 3));
        morse_delays.insert('/', (1, 7));

        let morse_player = MorsePlayer {
            _stream: Rc::new(stream),
            player: Arc::new(Mutex::new(sink)),
            cancellation_token: Rc::new(RefCell::new(CancellationToken::new())),
            actions: Rc::new(RefCell::new(morse_delays)),
            alphabet: Rc::new(RefCell::new(HashMap::new()))
        };

        morse_player.set_alphabet(Alphabet::Latin);

        morse_player
    }

    #[inline]
    pub fn timings(&self, text: &str, text_type: TextType, speed: u32, delay: u32) -> (Duration, Vec<Duration>) {
        self.actions.borrow_mut().insert('$', (1, delay));
        self.actions.borrow_mut().insert('/', (1, (delay as f32 * 2.3333) as u32));

        let text_preview = Self::gen_audio_prev_vec(&self.alphabet.borrow(), text);

        let (duration, timings) = Self::get_timings(
            text_preview,
            text_type,
            speed,
            &self.actions.borrow(),
        );

        return (duration, timings)
    }

    #[inline]
    pub fn set_volume(&self, volume: f32) {
        self.player.lock().unwrap().set_volume(volume);
    }

    #[inline]
    pub fn set_alphabet(&self, alphabet: Alphabet) {
        *self.alphabet.borrow_mut() = MORSE_CODE.get(&alphabet.to_string()).unwrap().clone();
        if alphabet != Alphabet::Latin {
            self.alphabet.borrow_mut().extend(MORSE_CODE.get(&Alphabet::Latin.to_string()).unwrap().clone());
        }
    }

    #[inline]
    pub fn stop(&self) {
        self.cancellation_token.borrow().cancel();
        self.player.lock().unwrap().clear();
    }

    #[inline]
    pub fn play(&self, text: &str, text_type: TextType, speed: u32, delay: u32, frequency: f32, wave_type: WaveType, sample_rate: u32) {
        self.actions.borrow_mut().insert('$', (1, delay));
        self.actions.borrow_mut().insert('/', (1, (delay as f32 * 2.3333) as u32));

        let player = self.player.clone();
        let actions = self.actions.borrow().clone();
        let cancellation_token = CancellationToken::new();
        *self.cancellation_token.borrow_mut() = cancellation_token.clone();

        player.lock().unwrap().play();

        let text_preview = Self::gen_audio_prev_vec(&self.alphabet.borrow(), text);

        std::thread::spawn(move || {
            Self::play_audio(
                text_preview,
                text_type,
                player,
                cancellation_token,
                actions,
                frequency,
                sample_rate,
                speed,
                wave_type,
            );
        });
    }

    pub fn get_morse(&self, letter: &char) -> String {
        self.alphabet.borrow().get(letter).unwrap().clone()
    }

    fn apply_fade_in(samples: &mut Vec<f32>, samples_count: usize) {
        for i in 0..samples_count {
            let scale = 0.5 * (1.0 - (std::f32::consts::PI * i as f32 / (samples_count as f32 - 1.0)).cos());
            samples[i] *= scale;
        }
    }

    fn apply_fade_out(samples: &mut Vec<f32>, samples_count: usize) {
        let len = samples.len();
        for i in 0..samples_count {
            let scale = 0.5 * (1.0 - (std::f32::consts::PI * i as f32 / (samples_count as f32 - 1.0)).cos());
            samples[len - 1 - i] *= scale;
        }
    }

    fn get_wave(generator: &WaveGenerator, sample_rate: u32, duration: Duration) -> Vec<f32> {
        let samples_wave_count = (sample_rate as f32 * duration.as_secs_f32()).round() as usize;
        let mut wave: Vec<f32> = (0..samples_wave_count).map(|_| generator.next()).collect();

        Self::apply_fade_in(&mut wave, (sample_rate as f32 * FADE_IN).round() as usize);
        Self::apply_fade_out(&mut wave, (sample_rate as f32 * FADE_OUT).round() as usize);

        wave
    }

    fn get_silence(generator: &WaveGenerator, sample_rate: u32, duration: Duration) -> Vec<f32> {
        let samples_count = (sample_rate as f32 * duration.as_secs_f32()).round() as usize;
        generator.skip_samples(samples_count);
        vec![0.0; samples_count]
    }

    fn play_audio(
        text: Vec<char>,
        text_type: TextType,
        player: Arc<Mutex<Player>>,
        cancellation_token: CancellationToken,
        actions: HashMap<char, (u8, u32)>,
        frequency: f32,
        sample_rate: u32,
        speed: u32,
        wave_type: WaveType,
    ) {
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            let generator = WaveGenerator::new(wave_type, frequency, sample_rate);
            let mut sound_signal = Vec::<f32>::new();
            let mut samples_duration = Duration::from_secs(0);

            let dot_duration = Self::get_dot_duration(text_type, speed as f64);
            let short_wave_length = dot_duration * actions.get(&'.').unwrap().1;
            let long_wave_length = dot_duration * actions.get(&'-').unwrap().1;
            let short_silence_length = dot_duration * actions.get(&'*').unwrap().1;
            let medium_silence_length = dot_duration * actions.get(&'$').unwrap().1;
            let long_silence_length = dot_duration * actions.get(&'/').unwrap().1;

            for (i, element) in text.iter().enumerate() {
                match actions.get(&element) {
                    Some(action) => {
                        if action.0 == 0 {
                            if element == &'.' {
                                sound_signal.extend(Self::get_wave(&generator, sample_rate, short_wave_length));
                            }
                            else {
                                sound_signal.extend(Self::get_wave(&generator, sample_rate, long_wave_length));
                            }
                        }
                        else if action.0 == 1 {
                            if element == &'*' { // Pause between dots or dashes
                                sound_signal.extend(Self::get_silence(&generator, sample_rate, short_silence_length));
                            }
                            else if element == &'$' { // Pause between characters
                                sound_signal.extend(Self::get_silence(&generator, sample_rate, medium_silence_length));
                            }
                            else { // Pause between words
                                sound_signal.extend(Self::get_silence(&generator, sample_rate, long_silence_length));
                            }
                        }
                    },
                    _none => { },
                }

                if *element == '/' || i+1 == text.len() {
                    if cancellation_token.is_cancelled() {
                        return
                    }
                    if player.lock().unwrap().len() > SINK_BUFFER_SIZE as usize {
                        tokio::select! {
                            _ = tokio::time::sleep(samples_duration) => { }
                            _ = cancellation_token.cancelled() => {
                                return
                            }
                        }
                    }
                    player.lock().unwrap().append(rodio::buffer::SamplesBuffer::new(
                        NonZero::new(1).unwrap(),
                        NonZero::new(sample_rate).unwrap(),
                        sound_signal.to_vec()
                    ));
                    samples_duration = Duration::from_secs_f64(sound_signal.len() as f64 / sample_rate as f64);
                    sound_signal.clear();
                }
            }
        });
    }

    fn gen_audio_prev_vec(alphabet: &HashMap<char, String>, text: &str) -> Vec<char> {
        let mut audio_vec = Vec::<char>::new();
        let text_vec: Vec<char> = text.chars().collect();

        for (i, element) in text_vec.iter().enumerate() {
            if let Some(morse_code) = alphabet.get(&element) {
                for (n, morse_char) in morse_code.chars().enumerate() {
                    audio_vec.push(morse_char);
                    if n+1 != morse_code.len() {
                        audio_vec.push('*');
                    }
                }
            }

            if *element != ' ' && i != text_vec.len() - 1 && text_vec[i+1] != ' ' {
                audio_vec.push('$');
            }

            if *element == ' ' {
                audio_vec.push('/');
            }

            audio_vec.push('^');
        }
        
        return audio_vec;
    }

    fn get_dot_duration(text_type: TextType, speed: f64) -> Duration { // calculating an absolute speed
        let speed_to_use: f64;
        match text_type {
            TextType::Letters => {
                speed_to_use = LETTERS_DURATION * 100.0 / speed;
            }
            TextType::Digits => {
                speed_to_use = DIGITS_DURATION * 100.0 / speed;
            }
            TextType::Mixed => {
                speed_to_use = MIXED_DURATION * 100.0 / speed;
            }
        }
        Duration::from_secs_f64(speed_to_use)
    }

    fn get_timings(
        audio_prev_vec: Vec<char>,
        text_type: TextType,
        speed: u32,
        actions: &HashMap<char, (u8, u32)>,
        ) -> (Duration, Vec<Duration>) {
            let mut timings = Vec::<Duration>::new();
            let mut duration = Duration::from_secs(0);
            let dot_duration = Self::get_dot_duration(text_type, speed as f64);
            timings.push(duration);

            for element in audio_prev_vec {
                match actions.get(&element) {
                    Some(action) => {
                        let duration_multiplier = action.1;
                        duration += dot_duration * duration_multiplier;
                    }
                    _none => { },
                }
                if element == '^' {
                    timings.push(duration);
                }
            }

            (duration, timings)
        }
}

impl Default for MorsePlayer {
    fn default() -> Self {
        Self::new()
    }
}
