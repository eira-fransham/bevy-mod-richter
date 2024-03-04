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

mod item;

use bevy::{
    ecs::{
        system::{Commands, IntoSystem, Resource},
        world::World,
    },
    render::extract_resource::ExtractResource,
};
use failure::{bail, Error};

pub use self::item::{Enum, EnumItem, Item, Slider, TextField, Toggle};

#[derive(Default, Clone, Copy, Debug)]
pub enum MenuState {
    /// Menu is inactive.
    #[default]
    Inactive,

    /// Menu is active. `index` indicates the currently selected element.
    Active { index: usize },

    /// A submenu of this menu is active. `index` indicates the active submenu.
    InSubMenu { index: usize },
}

#[derive(Default, Debug, Clone)]
/// Specifies how the menu body should be rendered.
pub enum MenuBodyView {
    /// The menu body is rendered using a predefined bitmap.
    Predefined {
        /// The path to the bitmap.
        path: imstr::ImString,
    },
    /// The menu body is rendered dynamically based on its contents.
    #[default]
    Dynamic,
}

#[derive(Default, Debug, Clone)]
pub struct MenuView {
    pub draw_plaque: bool,
    pub title_path: String,
    pub body: MenuBodyView,
}

impl MenuView {
    /// Returns true if the Quake plaque should be drawn to the left of the menu.
    pub fn draw_plaque(&self) -> bool {
        self.draw_plaque
    }

    /// Returns the path to the menu title bitmap.
    pub fn title_path(&self) -> &str {
        &self.title_path
    }

    /// Returns a MenuBodyView which specifies how to render the menu body.
    pub fn body(&self) -> &MenuBodyView {
        &self.body
    }
}

#[derive(Default, Debug, Resource, ExtractResource, Clone)]
pub struct Menu {
    items: im::Vector<NamedMenuItem>,
    state: MenuState,
    view: MenuView,
}

impl Menu {
    /// Returns a reference to the active submenu of this menu and its parent.
    fn active_submenu_and_parent(&self) -> Result<(&Menu, Option<&Menu>), Error> {
        let mut m = self;
        let mut m_parent = None;

        while let MenuState::InSubMenu { index } = m.state {
            match m.items[index].item {
                Item::Submenu(ref s) => {
                    m_parent = Some(m);
                    m = s;
                }
                _ => bail!("Menu state points to invalid submenu"),
            }
        }

        Ok((m, m_parent))
    }

    /// Return a reference to the active submenu of this menu
    pub fn active_submenu(&self) -> Result<&Menu, Error> {
        let (m, _) = self.active_submenu_and_parent()?;
        Ok(m)
    }

    /// Return a reference to the parent of the active submenu of this menu.
    ///
    /// If this is the root menu, returns None.
    fn active_submenu_parent(&self) -> Result<Option<&Menu>, Error> {
        let (_, m_parent) = self.active_submenu_and_parent()?;
        Ok(m_parent)
    }

    /// Return a reference to the active submenu of this menu
    pub fn active_submenu_mut(&mut self) -> Result<&mut Menu, Error> {
        let mut m = self;

        while let MenuState::InSubMenu { index } = &mut m.state {
            match &mut m.items[*index].item {
                Item::Submenu(s) => {
                    m = s;
                }
                _ => bail!("Menu state points to invalid submenu"),
            }
        }

        Ok(m)
    }

    /// Returns a reference to the active submenu of this menu and its parent.
    fn active_submenu_parent_mut(&mut self) -> Result<Option<&mut Menu>, Error> {
        let MenuState::InSubMenu { mut index } = self.active_submenu()?.state else {
            return Ok(Some(self));
        };
        let Item::Submenu(m) = &mut self.items[index].item else {
            bail!("Menu state points to invalid submenu");
        };
        let mut m = m;

        loop {
            match &mut m.items[index].item {
                Item::Submenu(s) => {
                    m = s;
                    if let MenuState::InSubMenu { index: new_index } = m.state {
                        index = new_index;
                    } else {
                        return Ok(Some(m));
                    }
                }
                _ => bail!("Menu state points to invalid submenu"),
            }
        }
    }

    /// Select the next element of this Menu.
    pub fn next(&mut self) -> Result<(), Error> {
        let m = self.active_submenu_mut()?;

        if let MenuState::Active { index } = m.state {
            m.state = MenuState::Active {
                index: (index + 1) % m.items.len(),
            };
        } else {
            bail!("Selected menu is inactive (invariant violation)");
        }

        Ok(())
    }

    /// Select the previous element of this Menu.
    pub fn prev(&mut self) -> Result<(), Error> {
        let m = self.active_submenu_mut()?;

        if let MenuState::Active { index } = m.state {
            m.state = MenuState::Active {
                index: index
                    .checked_sub(1)
                    .map(|i| i % m.items.len())
                    .unwrap_or(m.items.len() - 1),
            };
        } else {
            bail!("Selected menu is inactive (invariant violation)");
        }

        Ok(())
    }

    /// Return a reference to the currently selected menu item.
    pub fn selected(&self) -> Result<&Item, Error> {
        let m = self.active_submenu()?;

        if let MenuState::Active { index } = m.state {
            Ok(&m.items[index].item)
        } else {
            bail!("Active menu in invalid state (invariant violation)");
        }
    }

    /// Activate the currently selected menu item.
    ///
    /// If this item is a `Menu`, sets the active (sub)menu's state to
    /// `MenuState::InSubMenu` and the selected submenu's state to
    /// `MenuState::Active`.
    ///
    /// If this item is an `Action`, executes the function contained in the
    /// `Action`.
    ///
    /// Otherwise, this has no effect.
    #[must_use]
    pub fn activate(&mut self) -> Result<Option<impl FnOnce(&mut Commands)>, Error> {
        let m = self.active_submenu_mut()?;

        if let MenuState::Active { index } = m.state {
            match &mut m.items[index].item {
                Item::Submenu(submenu) => {
                    m.state = MenuState::InSubMenu { index };
                    submenu.state = MenuState::Active { index: 0 };

                    Ok(None)
                }

                Item::Action(action) => {
                    let action = *action;
                    Ok(Some(move |c: &mut Commands| c.run_system(action)))
                }

                _ => Ok(None),
            }
        } else {
            Ok(None)
        }
    }

    #[must_use]
    pub fn left(&mut self) -> Result<Option<impl FnOnce(&mut Commands)>, Error> {
        let m = self.active_submenu_mut()?;

        Ok(if let MenuState::Active { index } = m.state {
            match &mut m.items[index].item {
                Item::Enum(e) => e
                    .select_prev()
                    .map(|f| Box::new(f) as Box<dyn FnOnce(&mut Commands)>),
                Item::Slider(slider) => slider.decrease().map(|f| Box::new(f) as _),
                Item::TextField(text) => {
                    text.cursor_left();
                    None
                }
                Item::Toggle(toggle) => toggle.set_false().map(|f| Box::new(f) as _),
                _ => None,
            }
        } else {
            None
        })
    }

    #[must_use]
    pub fn right(&mut self) -> Result<Option<impl FnOnce(&mut Commands)>, Error> {
        let m = self.active_submenu_mut()?;

        Ok(if let MenuState::Active { index } = m.state {
            match &mut m.items[index].item {
                Item::Enum(e) => e
                    .select_next()
                    .map(|f| Box::new(f) as Box<dyn FnOnce(&mut Commands)>),
                Item::Slider(slider) => slider.increase().map(|f| Box::new(f) as _),
                Item::TextField(text) => {
                    text.cursor_right();
                    None
                }
                Item::Toggle(toggle) => toggle.set_true().map(|f| Box::new(f) as _),
                _ => None,
            }
        } else {
            None
        })
    }

    /// Return `true` if the root menu is active, `false` otherwise.
    pub fn at_root(&self) -> bool {
        match self.state {
            MenuState::Active { .. } => true,
            _ => false,
        }
    }

    /// Deactivate the active menu and activate its parent
    pub fn back(&mut self) -> Result<(), Error> {
        if self.at_root() {
            bail!("Cannot back out of root menu!");
        }

        let m = self.active_submenu_mut()?;
        m.state = MenuState::Inactive;

        match self.active_submenu_parent_mut()? {
            Some(mp) => {
                let s = mp.state.clone();
                match s {
                    MenuState::InSubMenu { index } => mp.state = MenuState::Active { index },
                    _ => unreachable!(),
                };
            }

            None => unreachable!(),
        }

        Ok(())
    }

    pub fn items(&self) -> impl Iterator<Item = &NamedMenuItem> + '_ {
        self.items.iter()
    }

    pub fn state(&self) -> MenuState {
        self.state
    }

    pub fn view(&self) -> &MenuView {
        &self.view
    }
}

pub struct MenuBuilder<'a> {
    world: &'a mut World,
    gfx_name: Option<String>,
    items: im::Vector<NamedMenuItem>,
}

impl<'a> MenuBuilder<'a> {
    pub fn new(world: &'a mut World) -> Self {
        MenuBuilder {
            world,
            gfx_name: None,
            items: Default::default(),
        }
    }

    pub fn world(&mut self) -> &mut World {
        self.world
    }

    pub fn build(mut self, view: MenuView) -> Menu {
        // deactivate all child menus
        for item in self.items.iter_mut() {
            if let Item::Submenu(m) = &mut item.item {
                m.state = MenuState::Inactive;
            }
        }

        Menu {
            items: self.items,
            state: MenuState::Active { index: 0 },
            view,
        }
    }

    pub fn add_submenu<S>(
        mut self,
        name: S,
        submenu: impl FnOnce(MenuBuilder<'_>) -> Result<Menu, Error>,
    ) -> Result<Self, Error>
    where
        S: AsRef<str>,
    {
        let submenu = submenu(MenuBuilder::new(&mut *self.world))?;
        self.items
            .push_back(NamedMenuItem::new(name, Item::Submenu(submenu)));
        Ok(self)
    }

    pub fn add_action<N, S, M>(mut self, name: N, action: S) -> Self
    where
        N: AsRef<str>,
        S: IntoSystem<(), (), M> + 'static,
    {
        let action_id = self.world.register_system(action);
        self.items
            .push_back(NamedMenuItem::new(name, Item::Action(action_id)));
        self
    }

    pub fn add_toggle<N, M, S>(mut self, name: N, init: bool, on_toggle: S) -> Self
    where
        N: AsRef<str>,
        S: IntoSystem<bool, (), M> + 'static,
    {
        self.items.push_back(NamedMenuItem::new(
            name,
            Item::Toggle(Toggle::new(&mut self.world, init, on_toggle)),
        ));
        self
    }

    pub fn add_enum<S, E>(mut self, name: S, items: E, init: usize) -> Result<Self, Error>
    where
        S: AsRef<str>,
        E: FnOnce(EnumBuilder) -> Result<Vec<EnumItem>, Error>,
    {
        self.items.push_back(NamedMenuItem::new(
            name,
            Item::Enum(Enum::new(init, items(EnumBuilder::new(&mut *self.world))?)?),
        ));
        Ok(self)
    }

    pub fn add_slider<N, M, S>(
        mut self,
        name: N,
        min: f32,
        max: f32,
        steps: usize,
        init: usize,
        on_select: S,
    ) -> Result<Self, Error>
    where
        N: AsRef<str>,
        S: IntoSystem<f32, (), M> + 'static,
    {
        self.items.push_back(NamedMenuItem::new(
            name,
            Item::Slider(Slider::new(
                &mut *self.world,
                min,
                max,
                steps,
                init,
                on_select,
            )?),
        ));
        Ok(self)
    }

    pub fn add_text_field<N, M, S>(
        mut self,
        name: N,
        default: Option<N>,
        max_len: Option<usize>,
        on_update: S,
    ) -> Result<Self, Error>
    where
        N: AsRef<str>,
        S: IntoSystem<String, (), M> + 'static,
    {
        self.items.push_back(NamedMenuItem::new(
            name,
            Item::TextField(TextField::new(
                &mut *self.world,
                default,
                max_len,
                on_update,
            )?),
        ));
        Ok(self)
    }
}

pub struct EnumBuilder<'a> {
    world: &'a mut World,
    items: Vec<EnumItem>,
}

impl<'a> EnumBuilder<'a> {
    pub fn new(world: &'a mut World) -> Self {
        Self {
            world,
            items: Vec::new(),
        }
    }

    pub fn add_item<N, M, S>(mut self, name: N, on_select: S) -> Result<Self, Error>
    where
        N: AsRef<str>,
        S: IntoSystem<(), (), M> + 'static,
    {
        self.items
            .push(EnumItem::new(&mut *self.world, name, on_select)?);

        Ok(self)
    }

    pub fn build(self) -> Vec<EnumItem> {
        self.items
    }
}

#[derive(Debug, Clone)]
pub struct NamedMenuItem {
    name: String,
    item: Item,
}

impl NamedMenuItem {
    fn new<S>(name: S, item: Item) -> NamedMenuItem
    where
        S: AsRef<str>,
    {
        NamedMenuItem {
            name: name.as_ref().to_string(),
            item,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn item(&self) -> &Item {
        &self.item
    }
}

// #[cfg(test)]
// mod test {
//     use super::*;
//     use std::{cell::Cell, rc::Rc};

//     fn view() -> MenuView {
//         MenuView {
//             draw_plaque: false,
//             title_path: "path".to_string(),
//             body: MenuBodyView::Dynamic,
//         }
//     }

//     fn is_inactive(state: &MenuState) -> bool {
//         match state {
//             MenuState::Inactive => true,
//             _ => false,
//         }
//     }

//     fn is_active(state: &MenuState) -> bool {
//         match state {
//             MenuState::Active { .. } => true,
//             _ => false,
//         }
//     }

//     fn is_insubmenu(state: &MenuState) -> bool {
//         match state {
//             MenuState::InSubMenu { .. } => true,
//             _ => false,
//         }
//     }

//     #[test]
//     fn test_menu_builder() {
//         let action_target = Rc::new(Cell::new(false));
//         let action_target_handle = action_target.clone();

//         let _m = MenuBuilder::new()
//             .add_action("action", Box::new(move || action_target_handle.set(true)))
//             .build(view());

//         // TODO
//     }

//     #[test]
//     fn test_menu_active_submenu() {
//         let menu = MenuBuilder::new()
//             .add_submenu(
//                 "menu_1",
//                 MenuBuilder::new()
//                     .add_action("action_1", Box::new(|| ()))
//                     .build(view()),
//             )
//             .add_submenu(
//                 "menu_2",
//                 MenuBuilder::new()
//                     .add_action("action_2", Box::new(|| ()))
//                     .build(view()),
//             )
//             .build(view());

//         let m = &menu;
//         let m1 = match m.items[0].item {
//             Item::Submenu(ref m1i) => m1i,
//             _ => unreachable!(),
//         };
//         let m2 = match m.items[1].item {
//             Item::Submenu(ref m2i) => m2i,
//             _ => unreachable!(),
//         };

//         assert!(is_active(&m.state.get()));
//         assert!(is_inactive(&m1.state.get()));
//         assert!(is_inactive(&m2.state.get()));

//         // enter m1
//         m.activate().unwrap();
//         assert!(is_insubmenu(&m.state.get()));
//         assert!(is_active(&m1.state.get()));
//         assert!(is_inactive(&m2.state.get()));

//         // exit m1
//         m.back().unwrap();
//         assert!(is_active(&m.state.get()));
//         assert!(is_inactive(&m1.state.get()));
//         assert!(is_inactive(&m2.state.get()));

//         // enter m2
//         m.next().unwrap();
//         m.activate().unwrap();
//         assert!(is_insubmenu(&m.state.get()));
//         assert!(is_inactive(&m1.state.get()));
//         assert!(is_active(&m2.state.get()));
//     }
// }
