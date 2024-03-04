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

use bevy::{
    app::AppExit,
    ecs::{
        event::Events,
        system::{In, ResMut},
    },
};
use richter::client::menu::{Menu, MenuBodyView, MenuBuilder, MenuView};

use failure::Error;

pub fn build_main_menu(builder: MenuBuilder) -> Result<Menu, Error> {
    Ok(builder
        .add_submenu("Single Player", build_menu_sp)?
        .add_submenu("Multiplayer", build_menu_mp)?
        .add_submenu("Options", build_menu_options)?
        .add_action("Help/Ordering", || ())
        .add_action("Quit", |mut quit: ResMut<Events<AppExit>>| {
            quit.send(AppExit);
        })
        .build(MenuView {
            draw_plaque: true,
            title_path: "gfx/ttl_main.lmp".into(),
            body: MenuBodyView::Predefined {
                path: "gfx/mainmenu.lmp".into(),
            },
        }))
}

fn build_menu_sp(builder: MenuBuilder) -> Result<Menu, Error> {
    Ok(builder
        .add_action("New Game", || ())
        .add_action("Load", || unimplemented!())
        .add_action("Save", || unimplemented!())
        .build(MenuView {
            draw_plaque: true,
            title_path: "gfx/ttl_sgl.lmp".into(),
            body: MenuBodyView::Predefined {
                path: "gfx/sp_menu.lmp".into(),
            },
        }))
}

fn build_menu_mp(builder: MenuBuilder) -> Result<Menu, Error> {
    Ok(builder
        .add_submenu("Join a Game", build_menu_mp_join)?
        .add_action("New Game", || unimplemented!())
        .add_action("Setup", || unimplemented!())
        .build(MenuView {
            draw_plaque: true,
            title_path: "gfx/p_multi.lmp".into(),
            body: MenuBodyView::Predefined {
                path: "gfx/mp_menu.lmp".into(),
            },
        }))
}

fn build_menu_mp_join(builder: MenuBuilder) -> Result<Menu, Error> {
    Ok(builder
        .add_submenu("TCP", build_menu_mp_join_tcp)?
        // .add_textbox // description
        .build(MenuView {
            draw_plaque: true,
            title_path: "gfx/p_multi.lmp".into(),
            body: MenuBodyView::Predefined {
                path: "gfx/mp_menu.lmp".into(),
            },
        }))
}

fn build_menu_mp_join_tcp(builder: MenuBuilder) -> Result<Menu, Error> {
    // Join Game - TCP/IP          // title
    //
    //  Address: 127.0.0.1         // label
    //
    //  Port     [26000]           // text field
    //
    //  Search for local games...  // menu
    //
    //  Join game at:              // label
    //  [                        ] // text field
    Ok(builder
        // .add
        .add_toggle("placeholder", false, |In(_): In<bool>| ())
        .build(MenuView {
            draw_plaque: true,
            title_path: "gfx/p_multi.lmp".into(),
            body: MenuBodyView::Dynamic,
        }))
}

fn build_menu_options(builder: MenuBuilder) -> Result<Menu, Error> {
    Ok(builder
        // .add_submenu("Customize controls", unimplemented!())
        .add_action("Go to console", || ())
        .add_action("Reset to defaults", || ())
        .add_slider("Render scale", 0.25, 1.0, 2, 0, |In(_): In<f32>| ())?
        .add_slider("Screen Size", 0.0, 1.0, 10, 9, |In(_): In<f32>| ())?
        .add_slider("Brightness", 0.0, 1.0, 10, 9, |In(_): In<f32>| ())?
        .add_slider("Mouse Speed", 0.0, 1.0, 10, 9, |In(_): In<f32>| ())?
        .add_slider("CD music volume", 0.0, 1.0, 10, 9, |In(_): In<f32>| ())?
        .add_slider("Sound volume", 0.0, 1.0, 10, 9, |In(_): In<f32>| ())?
        .add_toggle("Always run", true, |In(_): In<bool>| ())
        .add_toggle("Invert mouse", false, |In(_): In<bool>| ())
        .add_toggle("Lookspring", false, |In(_): In<bool>| ())
        .add_toggle("Lookstrafe", false, |In(_): In<bool>| ())
        // .add_submenu("Video options", unimplemented!())
        .build(MenuView {
            draw_plaque: true,
            title_path: "gfx/p_option.lmp".into(),
            body: MenuBodyView::Dynamic,
        }))
}
