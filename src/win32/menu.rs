//! Menus: reading RT_MENU resources and the process menu table.

use super::resources::{self, ResId};
use std::collections::BTreeMap;
use std::sync::Mutex;

pub const MF_GRAYED: u32 = 0x01;
pub const MF_DISABLED: u32 = 0x02;
pub const MF_CHECKED: u32 = 0x08;
pub const MF_POPUP: u32 = 0x10;
pub const MF_END: u32 = 0x80;
pub const MF_BYPOSITION: u32 = 0x400;
pub const MF_SEPARATOR: u32 = 0x800;

#[derive(Clone)]
pub struct Item {
    pub id: u32,
    pub text: String,
    pub flags: u32,
    /// Submenu (HMENU) when it is a popup.
    pub sub: u32,
}

static MENUS: Mutex<BTreeMap<u32, Vec<Item>>> = Mutex::new(BTreeMap::new());
static NEXT: Mutex<u32> = Mutex::new(0x9000);

pub fn add(items: Vec<Item>) -> u32 {
    let mut n = NEXT.lock().unwrap();
    let h = *n;
    *n += 4;
    MENUS.lock().unwrap().insert(h, items);
    h
}

unsafe fn r16(a: u32) -> u32 {
    *(a as usize as *const u16) as u32
}

/// Reads the items of a (sub)menu at `p`; returns (items, next address).
unsafe fn parse(mut p: u32) -> (Vec<Item>, u32) {
    let mut items = Vec::new();
    loop {
        let flags = r16(p);
        p += 2;
        let item = if flags & MF_POPUP != 0 {
            let (text, np) = resources::wide_z(p);
            let (sub_items, np) = parse(np);
            p = np;
            Item { id: 0, text, flags, sub: add(sub_items) }
        } else {
            let id = r16(p);
            let (text, np) = resources::wide_z(p + 2);
            p = np;
            Item { id, text, flags, sub: 0 }
        };
        items.push(item);
        if flags & MF_END != 0 {
            break;
        }
    }
    (items, p)
}

/// Loads menu `name` (RT_MENU) from the module; returns the HMENU.
pub unsafe fn load(base: u32, name: &ResId) -> u32 {
    let Some((_, data, _)) = resources::find(base, &ResId::Id(resources::RT_MENU), name) else { return 0 };
    let header_size = r16(data + 2);
    let (items, _) = parse(data + 4 + header_size);
    add(items)
}

pub fn exists(h: u32) -> bool {
    MENUS.lock().unwrap().contains_key(&h)
}

pub fn items(h: u32) -> Vec<Item> {
    MENUS.lock().unwrap().get(&h).cloned().unwrap_or_default()
}

pub fn destroy(h: u32) -> bool {
    let removed = MENUS.lock().unwrap().remove(&h);
    if let Some(items) = &removed {
        for it in items {
            if it.sub != 0 {
                destroy(it.sub);
            }
        }
    }
    removed.is_some()
}

/// Applies `f` to the item (by id, searching submenus, or by position) and returns the result.
pub fn with_item<R>(menu: u32, id_or_pos: u32, by_pos: bool, f: impl FnOnce(&mut Item) -> R) -> Option<R> {
    let mut all = MENUS.lock().unwrap();
    if by_pos {
        return all.get_mut(&menu)?.get_mut(id_or_pos as usize).map(f);
    }
    // by id: searches `menu` and, recursively, the submenus
    let mut stack = vec![menu];
    while let Some(m) = stack.pop() {
        let Some(items) = all.get(&m) else { continue };
        if let Some(pos) = items.iter().position(|i| i.sub == 0 && i.id == id_or_pos) {
            return Some(f(&mut all.get_mut(&m).unwrap()[pos]));
        }
        stack.extend(items.iter().filter(|i| i.sub != 0).map(|i| i.sub));
    }
    None
}

pub fn create_empty() -> u32 {
    add(Vec::new())
}

pub fn append(h: u32, flags: u32, id_or_sub: u32, text: String) -> bool {
    let mut all = MENUS.lock().unwrap();
    let Some(items) = all.get_mut(&h) else { return false };
    let popup = flags & MF_POPUP != 0;
    items.push(Item { id: if popup { 0 } else { id_or_sub }, text, flags, sub: if popup { id_or_sub } else { 0 } });
    true
}
