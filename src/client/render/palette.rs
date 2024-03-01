use std::io::BufReader;

use crate::{
    client::render::{DiffuseData, FullbrightData},
    common::vfs::Vfs,
};

use beef::Cow;
use bevy::{asset::AssetLoader, prelude::*};
use byteorder::ReadBytesExt;

pub struct Palette {
    rgb: [[u8; 3]; 256],
}

impl Palette {
    pub fn new(data: &[u8]) -> Palette {
        if data.len() != 768 {
            panic!("Bad len for rgb data");
        }

        let mut rgb = [[0; 3]; 256];
        for color in 0..256 {
            for component in 0..3 {
                rgb[color][component] = data[color * 3 + component];
            }
        }

        Palette { rgb }
    }

    pub fn load<S>(vfs: &Vfs, path: S) -> Palette
    where
        S: AsRef<str>,
    {
        let mut data = BufReader::new(vfs.open(path).unwrap());

        let mut rgb = [[0u8; 3]; 256];

        for color in 0..256 {
            for component in 0..3 {
                rgb[color][component] = data.read_u8().unwrap();
            }
        }

        Palette { rgb }
    }

    // TODO: this will not render console characters correctly, as they use index 0 (black) to
    // indicate transparency.
    /// Translates a set of indices into a list of RGBA values and a list of fullbright values.
    pub fn translate(&self, indices: &[u8]) -> (DiffuseData, FullbrightData) {
        let mut rgba = Vec::with_capacity(indices.len() * 4);
        let mut fullbright = Vec::with_capacity(indices.len());

        for index in indices {
            match *index {
                0xFF => {
                    for _ in 0..4 {
                        rgba.push(0);
                        fullbright.push(0);
                    }
                }

                i => {
                    for component in 0..3 {
                        rgba.push(self.rgb[*index as usize][component]);
                    }
                    rgba.push(0xFF);
                    fullbright.push(if i > 223 { 0xFF } else { 0 });
                }
            }
        }

        (
            DiffuseData {
                rgba: Cow::Owned(rgba),
            },
            FullbrightData {
                fullbright: Cow::Owned(fullbright),
            },
        )
    }
}

pub struct PalettedImageLoader {
    pub palette: Palette,
}

impl AssetLoader for PalettedImageLoader {
    type Asset = Image;
    type Settings = Option<(u32, u32)>;
    type Error = failure::Error;

    fn load<'a>(
        &'a self,
        reader: &'a mut bevy::asset::io::Reader,
        settings: &'a Self::Settings,
        load_context: &'a mut bevy::asset::LoadContext,
    ) -> bevy::utils::BoxedFuture<'a, Result<Self::Asset, Self::Error>> {
        todo!()
    }

    fn extensions(&self) -> &[&str] {
        &["lmp"]
    }
}
