//! Where a drawer unrolls and how big it is (milestone 0.11 review).
//!
//! A drawer is a button that opens a panel of choices. The panel always unrolls away from the nearest screen
//! border, so a control in the bottom-right corner opens up and to the left and never off the screen. All
//! numbers are in points: the window's logical size after the interface scale is applied, which is what the
//! shell's layout works in. At a larger scale the same screen holds fewer points, so a drawer that no longer
//! fits is cut down to the room there is and scrolls, instead of running off the edge.
//!
//! Pure geometry, no drawing, so every case is a unit test.

/// A rectangle in points; `x`, `y` is the top-left corner.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.right() && py >= self.y && py <= self.bottom()
    }
}

/// The side of the button a drawer unrolls towards.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    pub fn is_vertical(self) -> bool {
        matches!(self, Dir::Up | Dir::Down)
    }
}

/// How the drawer lines up with its button across the direction it unrolls.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Align {
    /// Leading edges match (left edges for a drawer going up or down).
    #[default]
    Start,
    Center,
    /// Trailing edges match.
    End,
}

/// The size limits of a grid drawer. Cells are `cell_w` by `cell_h` points with `gap` between them; the
/// drawer is as many columns as there are items up to `max_cols`, and as many rows as needed up to
/// `max_rows`. More items than that scroll.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct GridSpec {
    pub max_cols: u32,
    pub max_rows: u32,
    pub cell_w: u32,
    pub cell_h: u32,
    pub gap: u32,
}

impl GridSpec {
    /// Visible `(columns, rows)` for `items` entries; always at least 1x1.
    pub fn size_for(&self, items: usize) -> (u32, u32) {
        let n = u32::try_from(items).unwrap_or(u32::MAX).max(1);
        let cols = n.min(self.max_cols.max(1));
        let rows = n.div_ceil(cols).min(self.max_rows.max(1));
        (cols, rows)
    }

    /// The panel's content size in points for `items` entries.
    pub fn content_size(&self, items: usize) -> (f32, f32) {
        let (c, r) = self.size_for(items);
        (
            (c * self.cell_w + c.saturating_sub(1) * self.gap) as f32,
            (r * self.cell_h + r.saturating_sub(1) * self.gap) as f32,
        )
    }

    /// Columns per row, which is also how far Up and Down move in the item order.
    pub fn columns(&self, items: usize) -> usize {
        self.size_for(items).0 as usize
    }
}

/// The two shapes a drawer can take.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DrawerLayout {
    /// One column, as wide as the longest entry, `max_rows` tall at most before it scrolls.
    List {
        max_rows: u32,
    },
    Grid(GridSpec),
}

impl DrawerLayout {
    /// Items moved by Up/Down in the item order (1 for a list).
    pub fn columns(&self, items: usize) -> usize {
        match self {
            DrawerLayout::List { .. } => 1,
            DrawerLayout::Grid(g) => g.columns(items),
        }
    }
}

/// The side to unroll towards: opposite the screen border nearest to the button. A tie goes to the
/// vertical axis (up when the button is nearest the bottom, down when nearest the top), then to the
/// horizontal one.
pub fn unroll_dir(anchor: Rect, screen: Rect) -> Dir {
    let left = anchor.x - screen.x;
    let right = screen.right() - anchor.right();
    let top = anchor.y - screen.y;
    let bottom = screen.bottom() - anchor.bottom();
    let nearest = left.min(right).min(top).min(bottom);
    if bottom <= nearest {
        Dir::Up
    } else if top <= nearest {
        Dir::Down
    } else if right <= nearest {
        Dir::Left
    } else {
        Dir::Right
    }
}

/// Where a drawer goes.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Placement {
    pub dir: Dir,
    pub rect: Rect,
    /// The content was cut down to fit the screen, so the panel scrolls across the unroll direction.
    pub scrolls_along: bool,
    /// The content is wider (or taller) than the screen allows across the unroll direction.
    pub scrolls_across: bool,
}

/// Places a panel of `content` size next to `anchor` inside `screen`, `gap` away from the button and at
/// least `margin` from every border. The panel is cut down to the room that exists.
pub fn place(
    anchor: Rect,
    screen: Rect,
    content: (f32, f32),
    align: Align,
    gap: f32,
    margin: f32,
) -> Placement {
    let dir = unroll_dir(anchor, screen);
    let (cw, ch) = (content.0.max(1.0), content.1.max(1.0));
    let min_x = screen.x + margin;
    let max_x = (screen.right() - margin).max(min_x);
    let min_y = screen.y + margin;
    let max_y = (screen.bottom() - margin).max(min_y);

    // Along the unroll axis the room is what lies between the button and the border.
    let (w, h, x, y, along, across);
    if dir.is_vertical() {
        let room = if dir == Dir::Up {
            anchor.y - gap - min_y
        } else {
            max_y - (anchor.bottom() + gap)
        }
        .max(1.0);
        h = ch.min(room);
        w = cw.min(max_x - min_x);
        along = h < ch;
        across = w < cw;
        y = if dir == Dir::Up {
            anchor.y - gap - h
        } else {
            anchor.bottom() + gap
        };
        x = cross_pos(anchor.x, anchor.w, w, align).clamp(min_x, (max_x - w).max(min_x));
    } else {
        let room = if dir == Dir::Left {
            anchor.x - gap - min_x
        } else {
            max_x - (anchor.right() + gap)
        }
        .max(1.0);
        w = cw.min(room);
        h = ch.min(max_y - min_y);
        along = w < cw;
        across = h < ch;
        x = if dir == Dir::Left {
            anchor.x - gap - w
        } else {
            anchor.right() + gap
        };
        y = cross_pos(anchor.y, anchor.h, h, align).clamp(min_y, (max_y - h).max(min_y));
    }
    Placement {
        dir,
        rect: Rect::new(x, y, w, h),
        scrolls_along: along,
        scrolls_across: across,
    }
}

fn cross_pos(start: f32, len: f32, size: f32, align: Align) -> f32 {
    match align {
        Align::Start => start,
        Align::Center => start + (len - size) / 2.0,
        Align::End => start + len - size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 1000.0,
        h: 600.0,
    };

    #[test]
    fn a_drawer_opens_away_from_the_nearest_border() {
        let at = |x, y| Rect::new(x, y, 80.0, 30.0);
        // Bottom edge nearest: up. Top edge nearest: down.
        assert_eq!(unroll_dir(at(450.0, 560.0), SCREEN), Dir::Up);
        assert_eq!(unroll_dir(at(450.0, 10.0), SCREEN), Dir::Down);
        // Left edge nearest: right. Right edge nearest: left.
        assert_eq!(unroll_dir(at(5.0, 300.0), SCREEN), Dir::Right);
        assert_eq!(unroll_dir(at(915.0, 300.0), SCREEN), Dir::Left);
        // The bottom-right corner (equally near) goes up, the vertical axis winning ties.
        assert_eq!(unroll_dir(at(910.0, 560.0), SCREEN), Dir::Up);
        assert_eq!(unroll_dir(at(10.0, 10.0), SCREEN), Dir::Down);
    }

    #[test]
    fn a_list_drawer_sits_beside_its_button_by_alignment_and_never_leaves_the_screen() {
        let anchor = Rect::new(900.0, 550.0, 80.0, 30.0);
        let p = place(anchor, SCREEN, (120.0, 140.0), Align::Start, 4.0, 8.0);
        assert_eq!(p.dir, Dir::Up);
        assert_eq!(p.rect.bottom(), 546.0, "the gap is kept");
        assert!(
            p.rect.right() <= 992.0,
            "start-aligned would overflow, so it is pushed in"
        );
        assert!(!p.scrolls_along && !p.scrolls_across);
        // End alignment lines up trailing edges (and stays on screen).
        let p = place(anchor, SCREEN, (120.0, 140.0), Align::End, 4.0, 8.0);
        assert_eq!(p.rect.right(), 980.0);
        // Centre alignment on a button that has room.
        let mid = Rect::new(400.0, 550.0, 80.0, 30.0);
        let p = place(mid, SCREEN, (120.0, 140.0), Align::Center, 4.0, 8.0);
        assert_eq!(p.rect.x, 440.0 - 60.0);
    }

    #[test]
    fn a_drawer_taller_than_the_room_is_cut_down_and_scrolls() {
        let anchor = Rect::new(900.0, 550.0, 80.0, 30.0);
        let p = place(anchor, SCREEN, (120.0, 900.0), Align::End, 4.0, 8.0);
        assert!(p.scrolls_along);
        assert_eq!(p.rect.y, 8.0);
        assert_eq!(p.rect.bottom(), 546.0);
    }

    #[test]
    fn a_larger_interface_scale_leaves_fewer_points_so_the_same_drawer_scrolls() {
        // The same 1000x600 pixel window at 100% and at 200%: 1000x600 points against 500x300.
        let content = (120.0, 260.0);
        let at100 = place(
            Rect::new(900.0, 550.0, 80.0, 30.0),
            SCREEN,
            content,
            Align::End,
            4.0,
            8.0,
        );
        assert!(!at100.scrolls_along);
        let small = Rect::new(0.0, 0.0, 500.0, 300.0);
        let at200 = place(
            Rect::new(400.0, 250.0, 80.0, 30.0),
            small,
            content,
            Align::End,
            4.0,
            8.0,
        );
        assert!(at200.scrolls_along);
        assert!(at200.rect.y >= 8.0 && at200.rect.bottom() <= 292.0);
        // Never wider than the screen either.
        let wide = place(
            Rect::new(400.0, 250.0, 80.0, 30.0),
            small,
            (900.0, 50.0),
            Align::Start,
            4.0,
            8.0,
        );
        assert!(wide.scrolls_across);
        assert!(wide.rect.x >= 8.0 && wide.rect.right() <= 492.0);
    }

    #[test]
    fn a_drawer_going_sideways_aligns_vertically() {
        let anchor = Rect::new(900.0, 290.0, 80.0, 30.0);
        let p = place(anchor, SCREEN, (150.0, 100.0), Align::Center, 4.0, 8.0);
        assert_eq!(p.dir, Dir::Left);
        assert_eq!(p.rect.right(), 896.0);
        assert_eq!(p.rect.y, 305.0 - 50.0);
        let left = Rect::new(5.0, 290.0, 80.0, 30.0);
        let p = place(left, SCREEN, (150.0, 100.0), Align::Start, 4.0, 8.0);
        assert_eq!(p.dir, Dir::Right);
        assert_eq!(p.rect.x, 89.0);
    }

    #[test]
    fn a_grid_sizes_itself_between_one_cell_and_its_maximum() {
        let g = GridSpec {
            max_cols: 4,
            max_rows: 3,
            cell_w: 40,
            cell_h: 30,
            gap: 4,
        };
        assert_eq!(g.size_for(0), (1, 1));
        assert_eq!(g.size_for(1), (1, 1));
        assert_eq!(g.size_for(3), (3, 1));
        assert_eq!(g.size_for(4), (4, 1));
        assert_eq!(g.size_for(7), (4, 2));
        assert_eq!(g.size_for(12), (4, 3));
        assert_eq!(g.size_for(500), (4, 3), "more items scroll");
        assert_eq!(
            g.content_size(7),
            (4.0 * 40.0 + 3.0 * 4.0, 2.0 * 30.0 + 4.0)
        );
        assert_eq!(g.columns(7), 4);
        assert_eq!(DrawerLayout::List { max_rows: 6 }.columns(9), 1);
        assert_eq!(DrawerLayout::Grid(g).columns(2), 2);
    }
}
