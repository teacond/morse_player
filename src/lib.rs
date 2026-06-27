// lib.rs
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

pub mod morse_player;
pub use morse_player::*;

#[cfg(test)]
mod tests {
    use std::time::Duration;

use super::*;

    #[test]
    fn test_text_duration() {
        let morse_player = MorsePlayer::new().unwrap();
        
        morse_player.set_alphabet(Alphabet::Latin);
        morse_player.set_text("ABCDE");
        morse_player.set_dot_duration(Duration::from_millis(50));
        morse_player.set_delay(3);

        let result = morse_player.timings().0;
        assert_eq!(result.as_secs_f64(), 2.25);
    }
}
