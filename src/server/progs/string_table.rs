use im::HashMap;
use imstr::string::{ImString, Local};
use std::{
    cell::{Ref, RefCell},
    ops::Deref,
};

use crate::server::progs::{ProgsError, StringId};

#[derive(Debug)]
pub struct StringTable {
    /// Interned string data.
    data: RefCell<ImString<Local>>,

    /// Caches string lengths for faster lookup.
    lengths: RefCell<HashMap<StringId, usize>>,
}

impl StringTable {
    pub fn new(data: Vec<u8>) -> StringTable {
        StringTable {
            data: ImString::from_utf8(data).unwrap().into(),
            lengths: RefCell::new(HashMap::new()),
        }
    }

    pub fn id_from_i32(&self, value: i32) -> Result<StringId, ProgsError> {
        if value < 0 {
            return Err(ProgsError::with_msg("id < 0"));
        }

        let id = StringId(value as usize);

        if id.0 < self.data.borrow().len() {
            Ok(id)
        } else {
            Err(ProgsError::with_msg(format!("no string with ID {}", value)))
        }
    }

    pub fn find<S>(&self, target: S) -> Option<StringId>
    where
        S: AsRef<str>,
    {
        let target = target.as_ref();
        for (ofs, _) in target.char_indices() {
            let sub = &self.data.borrow()[ofs..];
            if !sub.starts_with(target) {
                continue;
            }

            // Make sure the string is NUL-terminated. Otherwise, this could
            // erroneously return the StringId of a ImString whose first
            // `target.len()` bytes were equal to `target`, but which had
            // additional bytes.
            if sub.as_bytes().get(target.len()) != Some(&0) {
                continue;
            }

            return Some(StringId(ofs));
        }

        None
    }

    pub fn get(&self, id: StringId) -> Option<impl Deref<Target = str> + '_> {
        let start = id.0;

        if start >= self.data.borrow().len() {
            return None;
        }

        if let Some(len) = self.lengths.borrow().get(&id) {
            let end = start + len;
            return Some(Ref::map(self.data.borrow(), |this| &this[start..end]));
        }

        match self.data.borrow()[start..]
            .chars()
            .take(1024 * 1024)
            .enumerate()
            .find(|&(_i, c)| c == '\0')
        {
            Some((len, _)) => {
                self.lengths.borrow_mut().insert(id, len);
                let end = start + len;
                Some(Ref::map(self.data.borrow(), |this| &this[start..end]))
            }
            None => panic!("string data not NUL-terminated!"),
        }
    }

    pub fn insert<S>(&self, s: S) -> StringId
    where
        S: AsRef<str>,
    {
        let s = s.as_ref();

        assert!(!s.contains('\0'));

        let id = StringId(self.data.borrow().len());
        self.data.borrow_mut().push_str(s);
        self.lengths.borrow_mut().insert(id, s.len());
        id
    }

    pub fn find_or_insert<S>(&self, target: S) -> StringId
    where
        S: AsRef<str>,
    {
        match self.find(target.as_ref()) {
            Some(id) => id,
            None => self.insert(target),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = impl Deref<Target = str>> + '_ {
        // TODO: Make this work properly with the refcell - since the inner data
        //       is cheaply clonable this should be relatively easy.
        self.data
            .borrow()
            .split('\0')
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .into_iter()
    }
}
