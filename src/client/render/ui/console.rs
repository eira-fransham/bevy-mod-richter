use crate::{
    client::render::{
        ui::{
            glyph::{GlyphRendererCommand, GLYPH_HEIGHT, GLYPH_WIDTH},
            layout::{Anchor, AnchorCoord, Layout, ScreenPosition, Size},
            quad::{QuadRendererCommand, QuadTexture},
        },
        GraphicsState,
    },
    common::{
        console::{RenderConsoleInput, RenderConsoleOutput},
        vfs::Vfs,
        wad::QPic,
    },
};

use bevy::{
    prelude::*,
    render::renderer::{RenderDevice, RenderQueue},
};
use chrono::Duration;

const PAD_LEFT: i32 = GLYPH_WIDTH as i32;

pub struct ConsoleRenderer {
    conback: QuadTexture,
}

impl ConsoleRenderer {
    pub fn new(
        state: &GraphicsState,
        vfs: &Vfs,
        device: &RenderDevice,
        queue: &RenderQueue,
    ) -> ConsoleRenderer {
        let conback = QuadTexture::from_qpic(
            state,
            device,
            queue,
            &QPic::load(vfs.open("gfx/conback.lmp").unwrap()).unwrap(),
        );

        ConsoleRenderer { conback }
    }

    pub fn generate_commands<'a>(
        &'a self,
        output: &RenderConsoleOutput,
        input: &RenderConsoleInput,
        time: Duration,
        quad_cmds: &mut Vec<QuadRendererCommand<'a>>,
        glyph_cmds: &mut Vec<GlyphRendererCommand>,
        proportion: f32,
    ) {
        // TODO: take scale as cvar
        let scale = 2.0;
        let console_anchor = Anchor {
            x: AnchorCoord::Zero,
            y: AnchorCoord::Proportion(1.0 - proportion),
        };

        // draw console background
        quad_cmds.push(QuadRendererCommand {
            texture: &self.conback,
            layout: Layout {
                position: ScreenPosition::Absolute(console_anchor),
                anchor: Anchor::BOTTOM_LEFT,
                size: Size::DisplayScale { ratio: 1.0 },
            },
        });

        // draw version string
        let version_string = format!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        glyph_cmds.push(GlyphRendererCommand::Text {
            text: version_string,
            position: ScreenPosition::Absolute(console_anchor),
            anchor: Anchor::BOTTOM_RIGHT,
            scale,
        });

        // draw input line
        glyph_cmds.push(GlyphRendererCommand::Glyph {
            glyph_id: ']' as u8,
            position: ScreenPosition::Relative {
                anchor: console_anchor,
                x_ofs: PAD_LEFT,
                y_ofs: 0,
            },
            anchor: Anchor::BOTTOM_LEFT,
            scale,
        });
        // TODO: Implement colours
        glyph_cmds.push(GlyphRendererCommand::Text {
            text: input.cur_text.clone(),
            position: ScreenPosition::Relative {
                anchor: console_anchor,
                x_ofs: PAD_LEFT,
                y_ofs: 0,
            },
            anchor: Anchor::BOTTOM_LEFT,
            scale,
        });
        // blink cursor in half-second intervals
        // TODO: Reimplement cursor
        // if engine::duration_to_f32(time).fract() > 0.5 {
        //     glyph_cmds.push(GlyphRendererCommand::Glyph {
        //         glyph_id: 11,
        //         position: ScreenPosition::Relative {
        //             anchor: console_anchor,
        //             x_ofs: PAD_LEFT + (GLYPH_WIDTH * (input.cursor() + 1)) as i32,
        //             y_ofs: 0,
        //         },
        //         anchor: Anchor::BOTTOM_LEFT,
        //         scale,
        //     });
        // }

        let mut line_id = 0;
        let mut char_id = 0;

        // draw previous output
        for (_, chunk) in &output.text_chunks {
            let chunk = &chunk.text;
            // TODO: implement scrolling
            if line_id > 100 {
                break;
            }

            let mut chunk_out = String::new();
            for chr in chunk
                .chars()
                .filter(|c| c.is_ascii() && (c.is_ascii_whitespace() || !c.is_ascii_control()))
            {
                if chr == '\n' {
                    line_id += 1;
                    char_id = 0;
                    continue;
                }

                chunk_out.push(chr);

                let position = ScreenPosition::Relative {
                    anchor: console_anchor,
                    x_ofs: PAD_LEFT + (1 + char_id * GLYPH_WIDTH) as i32,
                    y_ofs: ((line_id + 1) * GLYPH_HEIGHT) as i32,
                };

                let c = if chr as u32 > std::u8::MAX as u32 {
                    warn!(
                        "char \"{}\" (U+{:4}) cannot be displayed in the console",
                        chr, chr as u32
                    );
                    '?'
                } else {
                    chr
                };

                glyph_cmds.push(GlyphRendererCommand::Glyph {
                    glyph_id: c as u8,
                    position,
                    anchor: Anchor::BOTTOM_LEFT,
                    scale,
                });

                char_id += 1;
            }
        }
    }
}
