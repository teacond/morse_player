// morse_player.rs
// Copyright (C) 2025  Jaŭhien Lavonćjeŭ <jauhien.lavoncjeu@gmail.com>
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

use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::{Arc, Mutex}, time::Duration};
use rodio::{OutputStream, OutputStreamBuilder, Sink};
use ndarray::Array1;
use tokio::runtime::Runtime;
use std::f32::consts::PI;
use tokio_util::sync::CancellationToken;
use derive_more::Debug;
use lazy_static::lazy_static;

lazy_static! {
    pub static ref MORSE_CODE: HashMap<char, &'static str> = {
        let morse_map: HashMap<char, &str> = [
            ('A', ".-"), ('B', "-..."), ('C', "-.-."), ('D', "-.."), ('E', "."),
            ('F', "..-."), ('G', "--."), ('H', "...."), ('I', ".."), ('J', ".---"),
            ('K', "-.-"), ('L', ".-.."), ('M', "--"), ('N', "-."), ('O', "---"),
            ('P', ".--."), ('Q', "--.-"), ('R', ".-."), ('S', "..."), ('T', "-"),
            ('U', "..-"), ('V', "...-"), ('W', ".--"), ('X', "-..-"), ('Y', "-.--"),
            ('Z', "--.."), ('0', "-----"), ('1', ".----"), ('2', "..---"), ('3', "...--"),
            ('4', "....-"), ('5', "....."), ('6', "-...."), ('7', "--..."), ('8', "---.."),
            ('9', "----."), ('.', ".-.-.-"), (',', "--..--"), ('/', "-..-."), ('?', "..--.."),
            ('=', "-...-"), ('+', ".-.-.")].iter().cloned().collect();
        morse_map
    };
}

const LETTERS_DURATION: f64 = 0.05;
const DIGITS_DURATION: f64 = 0.034;
const MIXED_DURATION: f64 = 0.042;
const FADE_IN: f32 = 0.0002;
const FADE_OUT: f32 = 0.0002;
const SINK_BUFFER_SIZE: u32 = 3;

#[derive(PartialEq, Debug, Default, Clone, Copy)]
pub enum TextType {
    #[default]
    Letters,
    Digits,
    Mixed,
}

#[derive(PartialEq, Debug, Default, Clone, Copy)]
pub enum WaveType {
    #[default]
    Square,
    Triangle,
    Sawtooth,
    Sine
}

#[derive(Clone, Debug)]
pub struct MorsePlayer {
    #[debug(skip)]
    _stream: Rc<OutputStream>,
    #[debug(skip)]
    sink: Arc<Mutex<Sink>>,
    cancellation_token: Rc<RefCell<CancellationToken>>,
    actions: Rc<RefCell<HashMap<char, (u8, u32)>>>,
}

impl MorsePlayer {
    #[inline]
    pub fn new() -> MorsePlayer {
        let stream = OutputStreamBuilder::open_default_stream().unwrap();
        let sink = Sink::connect_new(stream.mixer());
        sink.set_volume(0.5);
        let mut morse_delays = HashMap::new();
        morse_delays.insert('.', (0, 1));
        morse_delays.insert('-', (0, 3));
        morse_delays.insert('*', (1, 1));
        morse_delays.insert('$', (1, 3));
        morse_delays.insert('/', (1, 7));

        MorsePlayer {
            _stream: Rc::new(stream),
            sink: Arc::new(Mutex::new(sink)),
            cancellation_token: Rc::new(RefCell::new(CancellationToken::new())),
            actions: Rc::new(RefCell::new(morse_delays)),
        }
    }

    #[inline]
    pub fn timings(&self, text: &str, text_type: TextType, speed: u32, delay: u32) -> (Duration, Vec<Duration>) {
        self.actions.borrow_mut().insert('$', (1, delay));
        self.actions.borrow_mut().insert('/', (1, (delay as f32 * 2.3333) as u32));

        let text_preview = Self::gen_audio_prev_vec(text);

        let (duration, timings) = Self::get_timings(
            &text_preview,
            text_type,
            speed,
            &self.actions.borrow(),
        );
        return (duration, timings)
    }

    #[inline]
    pub fn set_volume(&self, volume: f32) {
        self.sink.lock().unwrap().set_volume(volume);
    }

    #[inline]
    pub fn stop(&self) {
        self.cancellation_token.borrow().cancel();
        self.sink.lock().unwrap().clear();
    }

    #[inline]
    pub fn play(&self, text: &str, text_type: TextType, speed: u32, delay: u32, frequency: f32, wave_type: WaveType, sample_rate: u32) {
        self.actions.borrow_mut().insert('$', (1, delay));
        self.actions.borrow_mut().insert('/', (1, (delay as f32 * 2.3333) as u32));

        let sink = self.sink.clone();
        let actions = self.actions.borrow().clone();
        let cancellation_token = CancellationToken::new();
        *self.cancellation_token.borrow_mut() = cancellation_token.clone();

        sink.lock().unwrap().play();

        let text_preview = Self::gen_audio_prev_vec(text);

        std::thread::spawn(move || {
            Self::play_audio(
                &text_preview,
                text_type,
                &sink,
                &cancellation_token,
                actions,
                frequency,
                sample_rate,
                speed,
                wave_type,
            );
        });
    }

    fn apply_fade_in(samples: &mut Array1<f32>, samples_count: usize) {
        for i in 0..samples_count {
            let scale = 0.5 * (1.0 - (std::f32::consts::PI * i as f32 / (samples_count as f32 - 1.0)).cos());
            samples[i] *= scale;
        }
    }

    fn apply_fade_out(samples: &mut Array1<f32>, samples_count: usize) {
        let len = samples.len();
        for i in 0..samples_count {
            let scale = 0.5 * (1.0 - (std::f32::consts::PI * i as f32 / (samples_count as f32 - 1.0)).cos());
            samples[len - 1 - i] *= scale;
        }
    }

    fn get_wave(wave_type: WaveType, sample_rate: u32, frequency: f32, duration: Duration) -> Vec<f32> {
        let samples_wave_count = sample_rate as f32 * duration.as_secs_f32();
        let t_wave = Array1::linspace(
            0.0,
            duration.as_secs_f32() - 1.0 / sample_rate as f32,
            samples_wave_count as usize
        );
        let two_pi_f = 2.0 * PI * frequency;
        let nyquist = sample_rate as f32 / 2.0;
        let max_k = ((nyquist / frequency + 1.0) / 2.0).floor() as usize;
        
        let mut wave = match wave_type {
            WaveType::Square => {
                let mut wave = Array1::zeros(t_wave.len());
                for k in 1..=max_k {
                    let harmonic = (2 * k - 1) as f32;
                    wave = wave + (two_pi_f * harmonic * &t_wave).mapv(f32::sin) / harmonic;
                }
                wave * (4.0 / PI)
            }
            WaveType::Triangle => {
                let mut wave = Array1::zeros(t_wave.len());
                for k in 0..max_k-1 {
                    let n = (2 * k + 1) as f32;
                    wave = wave + ((-1i32).pow(k as u32) as f32 / n.powi(2)) * (two_pi_f * n * &t_wave).mapv(f32::sin);
                }
                wave * (8.0 / PI.powi(2))
            }
            WaveType::Sawtooth => {
                let mut wave = Array1::zeros(t_wave.len());
                for k in 1..=max_k {
                    wave = wave + (-1i32).pow(k as  u32) as f32 * ((two_pi_f * k as f32 * &t_wave).mapv(f32::sin) / k as f32);
                }
                wave * (-2.0 / PI)
            }
            WaveType::Sine => {
                (2.0 * PI * frequency * t_wave).mapv(f32::sin)
            }
        };

        Self::apply_fade_in(&mut wave, (sample_rate as f32 * FADE_IN) as usize);
        Self::apply_fade_out(&mut wave, (sample_rate as f32 * FADE_OUT) as usize);

        // Wave normalization
        let max_amplitude = wave.iter().map(|x| x.abs()).fold(0.0, f32::max);
        if max_amplitude > 0.0 {
            wave /= max_amplitude;
        }

        wave.to_vec()
    }

    fn get_silence(sample_rate: u32, duration: Duration) -> Vec<f32> {
        let samples_count = sample_rate as f32 * duration.as_secs_f32();
        let silence: Vec<f32> = vec![0.0; samples_count as usize];
        silence
    }

    fn play_audio(
        text: &Vec<char>,
        text_type: TextType,
        sink: &Arc<Mutex<Sink>>,
        cancellation_token: &CancellationToken,
        actions: HashMap<char, (u8, u32)>,
        frequency: f32,
        sample_rate: u32,
        speed: u32,
        wave_type: WaveType,
    ) {
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            let mut sound_signal = Vec::<f32>::new();
            let dot_duration = Self::get_dot_duration(text_type, speed as f64);
            let short_wave_length = actions.get(&'.').unwrap().1;
            let long_wave_length = actions.get(&'-').unwrap().1;
            let short_wave = Self::get_wave(wave_type, sample_rate, frequency, dot_duration * short_wave_length);
            let long_wave = Self::get_wave(wave_type, sample_rate, frequency, dot_duration * long_wave_length);
            let short_silence = Self::get_silence(sample_rate, dot_duration * actions.get(&'*').unwrap().1 as u32);
            let medium_silence = Self::get_silence(sample_rate, dot_duration * actions.get(&'$').unwrap().1 as u32);
            let long_silence = Self::get_silence(sample_rate, dot_duration * actions.get(&'/').unwrap().1 as u32);
            let mut samples_duration = Duration::from_secs(0);

            for (i, element) in text.iter().enumerate() {
                match actions.get(&element) {
                    Some(action) => {
                        if action.0 == 0 {
                            if element == &'.' {
                                sound_signal.extend(short_wave.clone());
                            }
                            else {
                                sound_signal.extend(long_wave.clone());
                            }
                        }
                        else if action.0 == 1 {
                            if element == &'*' { // Pause between dots or dashes
                                sound_signal.extend(short_silence.clone());
                            }
                            else if element == &'$' { // Pause between characters
                                sound_signal.extend(medium_silence.clone());
                            }
                            else { // Pause between words
                                sound_signal.extend(long_silence.clone());
                            }
                        }
                    },
                    _none => { },
                }

                if *element == '/' || i+1 == text.len() {
                    if cancellation_token.is_cancelled() {
                        return
                    }
                    if sink.lock().unwrap().len() > SINK_BUFFER_SIZE as usize {
                        tokio::select! {
                            _ = tokio::time::sleep(samples_duration) => { }
                            _ = cancellation_token.cancelled() => {
                                return
                            }
                        }
                    }
                    sink.lock().unwrap().append(rodio::buffer::SamplesBuffer::new(1, sample_rate, sound_signal.to_vec()));
                    samples_duration = Duration::from_secs_f64(sound_signal.len() as f64 / sample_rate as f64);
                    sound_signal.clear();
                }
            }
        });
    }

    fn gen_audio_prev_vec(text: &str) -> Vec<char> {
        let mut audio_vec = Vec::<char>::new();
        let text_vec: Vec<char> = text.chars().collect();

        for (i, element) in text_vec.iter().enumerate() {
            if let Some(morse_code) = MORSE_CODE.get(&element) {
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
        audio_prev_vec: &Vec<char>,
        text_type: TextType,
        speed: u32,
        actions: &HashMap<char, (u8, u32)>,
        ) 
        -> (Duration, Vec<Duration>) {
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
                if *element == '^' {
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
