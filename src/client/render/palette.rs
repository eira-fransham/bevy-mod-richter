use std::{
    io::{self, BufReader},
    path::PathBuf,
};

use crate::{
    client::render::{DiffuseData, FullbrightData},
    common::vfs::Vfs,
};

use beef::Cow;
use bevy::{asset::AssetLoader, prelude::*, render::render_asset::RenderAssetUsages};
use byteorder::ReadBytesExt;
use futures::AsyncReadExt;
use serde::{Deserialize, Serialize};
use wgpu::{Extent3d, TextureDimension};

use super::{Extent2d, DIFFUSE_TEXTURE_FORMAT};

#[derive(Asset, TypePath, Debug)]
pub struct Palette {
    rgb: [[u8; 3]; 256],
}

#[derive(Default)]
struct PaletteLoader;

impl AssetLoader for PaletteLoader {
    type Asset = Palette;
    type Settings = ();
    type Error = std::io::Error;

    fn load<'a>(
        &'a self,
        reader: &'a mut bevy::asset::io::Reader,
        settings: &'a Self::Settings,
        load_context: &'a mut bevy::asset::LoadContext,
    ) -> bevy::utils::BoxedFuture<'a, Result<Self::Asset, Self::Error>> {
        Box::pin(async move {
            let mut rgb = [0u8; 3 * 256];
            reader.read_exact(&mut rgb).await.and_then(|rgb| {
                Ok(Palette {
                    rgb: bytemuck::cast(rgb),
                })
            })
        })
    }

    fn extensions(&self) -> &[&str] {
        &["lmp"]
    }
}

#[derive(Default)]
pub struct PalettedImageLoader;

#[derive(Serialize, Deserialize)]
pub struct PalettedImageLoaderSettings {
    pub transparent: u8,
    pub dimensions: Extent2d,
}

impl Default for PalettedImageLoaderSettings {
    fn default() -> Self {
        Self {
            transparent: 0xFF,
            dimensions: Extent2d {
                width: 256,
                height: 256,
            },
        }
    }
}

impl AssetLoader for PalettedImageLoader {
    type Asset = Image;
    type Settings = PalettedImageLoaderSettings;
    type Error = io::Error;

    fn load<'a>(
        &'a self,
        reader: &'a mut bevy::asset::io::Reader,
        settings: &'a Self::Settings,
        load_context: &'a mut bevy::asset::LoadContext,
    ) -> bevy::utils::BoxedFuture<'a, Result<Self::Asset, Self::Error>> {
        Box::pin(async move {
            let Some(palette_path) = load_context.asset_path().label() else {
                return Err(io::Error::from(io::ErrorKind::NotFound));
            };
            let palette_path = palette_path.to_owned();

            let palette = load_context
                .load_direct(PathBuf::from(palette_path))
                .await
                .map_err(|_| io::Error::from(io::ErrorKind::NotFound))?;
            let palette = palette
                .get::<Palette>()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
            let mut data_buf =
                vec![0u8; (settings.dimensions.width * settings.dimensions.height) as usize];
            let image = reader.read_exact(&mut data_buf).await?;
            let data = data_buf
                .into_iter()
                .flat_map(move |i| {
                    if i == settings.transparent {
                        [0u8; 4]
                    } else {
                        let [r, g, b] = palette.rgb[i as usize];
                        [r, g, b, 0xff]
                    }
                })
                .collect();

            Ok(Image::new(
                Extent3d {
                    width: settings.dimensions.width,
                    height: settings.dimensions.height,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                data,
                DIFFUSE_TEXTURE_FORMAT,
                RenderAssetUsages::RENDER_WORLD,
            ))
        })
    }

    fn extensions(&self) -> &[&str] {
        &["lmp"]
    }
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
                rgba: Cow::owned(rgba),
            },
            FullbrightData {
                fullbright: Cow::owned(fullbright),
            },
        )
    }
}
