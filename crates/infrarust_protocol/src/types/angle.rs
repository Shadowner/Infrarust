use crate::types::traits::{ProtocolRead, ProtocolWrite};
use std::f32::consts::PI;
use std::io::{self, Read, Write};

const FULL_ROTATION: f32 = 256.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Angle(pub u8);

impl Angle {
    pub fn from_degrees(degrees: f32) -> Self {
        // Normalize the angle to be between 0 and 360
        let normalized_degrees = degrees % 360.0;
        let normalized_degrees = if normalized_degrees < 0.0 {
            normalized_degrees + 360.0
        } else {
            normalized_degrees
        };

        // Use rounding instead of truncation for more accurate conversion
        let angle = ((normalized_degrees * FULL_ROTATION / 360.0).round()) as u8;
        Angle(angle)
    }

    pub fn from_radians(radians: f32) -> Self {
        let degrees = radians * 180.0 / PI;
        Self::from_degrees(degrees)
    }

    pub fn to_degrees(&self) -> f32 {
        (self.0 as f32) * 360.0 / FULL_ROTATION
    }

    pub fn to_radians(&self) -> f32 {
        self.to_degrees() * PI / 180.0
    }
}

impl ProtocolWrite for Angle {
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        writer.write_all(&[self.0])?;
        Ok(1)
    }
}

impl ProtocolRead for Angle {
    fn read_from<R: Read>(reader: &mut R) -> io::Result<(Self, usize)> {
        let mut buf = [0u8; 1];
        reader.read_exact(&mut buf)?;
        Ok((Angle(buf[0]), 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_angle_protocol() {
        let test_angles = vec![
            Angle(0),
            Angle(64),  // 90 degrees
            Angle(128), // 180 degrees
            Angle(192), // 270 degrees
            Angle(255), // ~359 degrees
        ];

        for angle in test_angles {
            let mut buffer = Vec::new();
            let written = angle.write_to(&mut buffer).unwrap();

            let mut cursor = Cursor::new(buffer);
            let (read_angle, read) = Angle::read_from(&mut cursor).unwrap();

            assert_eq!(written, read);
            assert_eq!(angle, read_angle);
        }
    }

    #[test]
    fn test_angle_conversions() {
        let test_cases = vec![
            (0.0, 0),
            (90.0, 64),
            (180.0, 128),
            (270.0, 192),
            (360.0, 0),
            // Add edge cases
            (359.9, 255),
            (0.1, 0),
            (45.0, 32),
        ];

        for (degrees, expected) in test_cases {
            let angle = Angle::from_degrees(degrees);
            assert_eq!(angle.0, expected);

            let back_to_degrees = angle.to_degrees();
            // Increase tolerance to account for quantization error
            assert!(
                (back_to_degrees - (degrees % 360.0)).abs() < 2.0,
                "Conversion failed for {} degrees: got {} degrees back",
                degrees,
                back_to_degrees
            );
        }
    }

    #[test]
    fn test_radian_conversion() {
        let angle = Angle::from_radians(PI);
        assert_eq!(angle.0, 128); // Should be ~180 degrees

        let back_to_radians = angle.to_radians();
        assert!((back_to_radians - PI).abs() < 0.1);
    }
}
