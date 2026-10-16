use std::{
    cell::RefCell,
    collections::VecDeque,
    path::{Path, PathBuf},
    rc::Rc,
};

use log::warn;
use smithay_client_toolkit::reexports::client::{protocol::wl_surface::WlSurface, QueueHandle};

use crate::{
    filelist_cache::FilelistCache,
    wallpaper_groups::{WallpaperGroup, WallpaperGroups},
    wallpaper_info::{Recursive, Sorting, WallpaperInfo},
    wpaperd::Wpaperd,
};

/// Result from `get_image_from_path` - distinguishes forced images from list-based images.
/// This makes the contract explicit: forced images have no meaningful index.
#[derive(Debug, Clone)]
pub enum ImageResult {
    /// Image was forced via `wpaperctl set` - not part of normal navigation
    Forced(PathBuf),
    /// Image from the configured directory/list with its index
    FromList { path: PathBuf, index: Option<usize> },
}

impl ImageResult {
    pub fn path(&self) -> &Path {
        match self {
            ImageResult::Forced(p) => p,
            ImageResult::FromList { path, .. } => path,
        }
    }
}

#[derive(Debug)]
pub struct Queue {
    buffer: VecDeque<PathBuf>,
    current: usize,
    tail: usize,
    size: usize,
}

impl Queue {
    pub fn with_capacity(size: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(size),
            current: 0,
            tail: size - 1,
            size,
        }
    }

    #[cfg(test)]
    fn current(&self) -> &Path {
        &self.buffer[self.current]
    }

    fn next(&mut self, wrap: bool) -> Option<&Path> {
        let next = (self.current + 1) % self.size;
        if !self.is_full() {
            if next < self.buffer.len() {
                self.current = next;
            } else if wrap {
                self.current = (self.current + 1) % self.buffer.len();
            } else {
                return None;
            }
        } else if self.current != self.tail {
            self.current = next;
        } else {
            return None;
        }
        Some(&self.buffer[self.current])
    }

    fn previous(&mut self, wrap: bool) -> Option<&Path> {
        let prev = (self.current + self.size - 1) % self.size;
        if !self.is_full() {
            if prev < self.buffer.len() {
                self.current = prev;
            } else if wrap {
                self.current = (self.current + self.buffer.len() - 1) % self.buffer.len();
            } else {
                return None;
            }
        } else if prev != self.tail {
            self.current = prev;
        } else {
            return None;
        }
        Some(&self.buffer[self.current])
    }

    fn is_full(&self) -> bool {
        self.buffer.len() == self.size
    }

    fn contains(&self, p: &PathBuf) -> bool {
        self.buffer.contains(p)
    }

    fn set_current_to(&mut self, p: &Path) {
        if let Some(index) = self.buffer.iter().position(|path| p == path) {
            self.current = index;
        }
    }

    fn push(&mut self, p: PathBuf) {
        // Avoid duplicates
        if self.buffer.contains(&p) {
            return;
        };

        if self.is_full() {
            self.buffer.pop_front();
            self.buffer.push_back(p);
        } else {
            self.buffer.push_back(p);
            self.current = self.buffer.len() - 1;
        }
    }

    fn resize(&mut self, new_size: usize) {
        if !self.is_full() {
            self.buffer.reserve_exact(new_size);
            self.size = new_size;
        } else {
            let relative_current = (self.current + self.size - self.tail) % self.size;
            self.current = (self.tail + self.size - new_size % self.size) % self.size;
            let relative_current = (relative_current + self.size - self.current - 1) % new_size;
            let mut new_buf = VecDeque::new();
            while let Some(prev) = self.next(false) {
                new_buf.push_back(prev.to_path_buf());
            }
            self.current = relative_current;
            self.tail = new_size - 1;
            self.size = new_size;
            self.buffer = new_buf;
        }
    }

    fn len(&self) -> usize {
        self.buffer.len()
    }
}

enum ImagePickerAction {
    Next,
    Previous,
}

struct GroupedRandom {
    surface: WlSurface,
    group: Rc<RefCell<WallpaperGroup>>,
    groups: Rc<RefCell<WallpaperGroups>>,
}

impl GroupedRandom {
    fn new(
        groups: Rc<RefCell<WallpaperGroups>>,
        group: u8,
        wl_surface: &WlSurface,
        queue_size: usize,
    ) -> Self {
        Self {
            surface: wl_surface.clone(),
            group: groups
                .borrow_mut()
                .get_or_insert(group, wl_surface, queue_size),
            groups: groups.clone(),
        }
    }
}

impl Drop for GroupedRandom {
    fn drop(&mut self) {
        let group = self.group.borrow();
        let group_index = group.group;
        drop(group);
        self.groups.borrow_mut().remove(group_index, &self.surface);
    }
}

enum ImagePickerSorting {
    Random(Queue),
    GroupedRandom(GroupedRandom),
    Ascending(usize),
    Descending(usize),
}

impl ImagePickerSorting {
    fn new(
        wallpaper_info: &WallpaperInfo,
        wl_surface: &WlSurface,
        groups: Rc<RefCell<WallpaperGroups>>,
        filelist_cache: Rc<RefCell<FilelistCache>>,
    ) -> Self {
        match wallpaper_info.sorting {
            None | Some(Sorting::Random) => {
                Self::new_random(wallpaper_info.drawn_images_queue_size)
            }
            Some(Sorting::GroupedRandom { group }) => {
                ImagePickerSorting::GroupedRandom(GroupedRandom::new(
                    groups,
                    group,
                    wl_surface,
                    wallpaper_info.drawn_images_queue_size,
                ))
            }
            Some(Sorting::Ascending) => {
                let files_len = filelist_cache
                    .clone()
                    .borrow()
                    .get(
                        &wallpaper_info.path,
                        wallpaper_info.recursive.unwrap_or_default(),
                    )
                    .len();
                Self::new_ascending(files_len)
            }
            Some(Sorting::Descending) => Self::new_descending(),
        }
    }

    fn new_random(queue_size: usize) -> Self {
        Self::Random(Queue::with_capacity(queue_size))
    }

    fn new_descending() -> ImagePickerSorting {
        Self::Descending(0)
    }

    fn new_ascending(files_len: usize) -> ImagePickerSorting {
        Self::Ascending(files_len.saturating_sub(1))
    }
}

pub struct ImagePicker {
    current_img: PathBuf,
    actions: VecDeque<ImagePickerAction>,
    sorting: ImagePickerSorting,
    filelist_cache: Rc<RefCell<FilelistCache>>,
    reload: bool,
    /// Forced image path from `wpaperctl set`, consumed on next get_image_from_path call
    forced_image: Option<PathBuf>,
    /// True if the currently displayed image was set via `wpaperctl set`
    was_last_forced: bool,
    /// `was_last_forced` before a forced image was taken, restored if that image fails to load
    forced_fallback: bool,
}

impl ImagePicker {
    pub const DEFAULT_DRAWN_IMAGES_QUEUE_SIZE: usize = 10;
    pub fn new(
        wallpaper_info: &WallpaperInfo,
        wl_surface: &WlSurface,
        filelist_cache: Rc<RefCell<FilelistCache>>,
        groups: Rc<RefCell<WallpaperGroups>>,
    ) -> Self {
        Self {
            current_img: PathBuf::from(""),
            actions: VecDeque::from([ImagePickerAction::Next]),
            sorting: ImagePickerSorting::new(
                wallpaper_info,
                wl_surface,
                groups,
                filelist_cache.clone(),
            ),
            filelist_cache,
            reload: false,
            forced_image: None,
            was_last_forced: false,
            forced_fallback: false,
        }
    }

    /// Get the next image based on the sorting method
    fn get_image_path(&mut self, files: &[PathBuf]) -> (Option<usize>, PathBuf) {
        match (self.actions.front(), &mut self.sorting) {
            (
                None,
                ImagePickerSorting::Ascending(current_index)
                | ImagePickerSorting::Descending(current_index),
            ) if self.current_img.exists() => {
                (Some(*current_index), self.current_img.to_path_buf())
            }
            (_, ImagePickerSorting::GroupedRandom(group))
                if group.group.borrow().loading_image.is_some() =>
            {
                let group = group.group.borrow();
                let loading_image = group.loading_image.as_ref().unwrap();
                (None, loading_image.to_path_buf())
            }
            (_, ImagePickerSorting::GroupedRandom(group))
                if group.group.borrow().current_image != self.current_img =>
            {
                let group = group.group.borrow();
                (None, group.current_image.clone())
            }
            (None, ImagePickerSorting::Random(_) | ImagePickerSorting::GroupedRandom(_))
                if self.current_img.exists() =>
            {
                (None, self.current_img.to_path_buf())
            }
            (None | Some(ImagePickerAction::Next), ImagePickerSorting::Random(queue)) => {
                (None, next_random_image(&self.current_img, queue, files))
            }
            (None | Some(ImagePickerAction::Next), ImagePickerSorting::GroupedRandom(group)) => {
                let mut group = group.group.borrow_mut();
                if self.current_img == group.current_image {
                    // start loading a new image
                    let path = next_random_image(&self.current_img, &mut group.queue, files);
                    group.loading_image = Some(path.to_path_buf());
                    (None, path)
                } else {
                    (None, group.current_image.clone())
                }
            }
            (Some(ImagePickerAction::Previous), ImagePickerSorting::Random(queue)) => (
                None,
                get_previous_image_for_random(&self.current_img, queue, files.len() <= queue.len()),
            ),
            (Some(ImagePickerAction::Previous), ImagePickerSorting::GroupedRandom(group)) => {
                let mut group = group.group.borrow_mut();
                let queue = &mut group.queue;
                let path = get_previous_image_for_random(
                    &self.current_img,
                    queue,
                    files.len() <= queue.len(),
                );
                if path != group.current_image {
                    group.loading_image = Some(path.to_path_buf());
                }
                (None, path)
            }
            (
                None | Some(ImagePickerAction::Next),
                ImagePickerSorting::Descending(current_index),
            )
            | (Some(ImagePickerAction::Previous), ImagePickerSorting::Ascending(current_index)) => {
                let index = if files.get(*current_index) == Some(&self.current_img) {
                    *current_index
                } else {
                    // if the current img doesn't correspond to the index we have
                    // try looking for it in files
                    match files.binary_search(&self.current_img) {
                        Ok(new_index) => new_index,
                        Err(_err) => {
                            // if we don't find it, use the last index as starting point
                            // if the current_index is too big, start from last image
                            // this is a fail safe in case many files gets deleted
                            if *current_index >= files.len() {
                                0
                            } else {
                                *current_index
                            }
                        }
                    }
                };
                let index = if index == 0 {
                    files.len() - 1
                } else {
                    index - 1
                };
                (Some(index), files[index].to_path_buf())
            }
            (Some(ImagePickerAction::Previous), ImagePickerSorting::Descending(current_index))
            | (
                None | Some(ImagePickerAction::Next),
                ImagePickerSorting::Ascending(current_index),
            ) => {
                let index = if files.get(*current_index) == Some(&self.current_img) {
                    *current_index
                } else {
                    match files.binary_search(&self.current_img) {
                        Ok(new_index) => new_index,
                        Err(_err) => *current_index,
                    }
                };
                let index = (index + 1) % files.len();
                (Some(index), files[index].to_path_buf())
            }
        }
    }

    /// Set the wallpaper to a specific image path.
    /// The forced image will be returned by the next call to `get_image_from_path`.
    pub fn set_image(&mut self, path: PathBuf) {
        self.forced_image = Some(path);
    }

    pub fn get_image_from_path(
        &mut self,
        path: &Path,
        recursive: &Option<Recursive>,
    ) -> Option<ImageResult> {
        // Check for forced image first (from wpaperctl set)
        // Don't update navigation state - forced images are "detours"
        if let Some(forced_path) = self.forced_image.take() {
            self.forced_fallback = self.was_last_forced;
            self.was_last_forced = true;
            return Some(ImageResult::Forced(forced_path));
        }

        if path.is_dir() {
            let files = self
                .filelist_cache
                .borrow()
                .get(path, recursive.unwrap_or_default());

            // There are no images, forcefully break out of the loop
            if files.is_empty() {
                warn!("Directory {path:?} does not contain any valid image files.");
                None
            } else {
                let (index, img_path) = self.get_image_path(&files);
                if img_path == self.current_img && !self.reload {
                    None
                } else {
                    Some(ImageResult::FromList {
                        path: img_path,
                        index,
                    })
                }
            }
        } else if path == self.current_img && !self.reload {
            None
        } else {
            // path is not a directory, also it's not the current image or we need to reload
            Some(ImageResult::FromList {
                path: path.to_path_buf(),
                index: None,
            })
        }
    }

    pub fn update_current_image(&mut self, result: ImageResult) {
        match result {
            ImageResult::Forced(img_path) => {
                // Don't update navigation state for forced images - they're "detours"
                // Clear action without updating state
                self.actions.clear();
                self.current_img = img_path;
            }
            ImageResult::FromList {
                path: img_path,
                index,
            } => {
                // A list image replaces the detour. An idle load pass that picks nothing new
                // must not clear it.
                self.was_last_forced = false;
                match (self.actions.pop_front(), &mut self.sorting) {
                    (Some(ImagePickerAction::Next), ImagePickerSorting::Random(queue)) => {
                        queue.push(img_path.clone());
                    }
                    (
                        None | Some(ImagePickerAction::Previous),
                        ImagePickerSorting::Random { .. },
                    ) => {}
                    (
                        None | Some(ImagePickerAction::Previous),
                        ImagePickerSorting::GroupedRandom(group),
                    ) => {
                        let mut group = group.group.borrow_mut();
                        group.loading_image = None;
                        group.current_image.clone_from(&img_path);
                    }
                    (
                        _,
                        ImagePickerSorting::Ascending(current_index)
                        | ImagePickerSorting::Descending(current_index),
                    ) => *current_index = index.unwrap_or(usize::MAX),
                    (Some(ImagePickerAction::Next), ImagePickerSorting::GroupedRandom(group)) => {
                        let mut group = group.group.borrow_mut();
                        let queue = &mut group.queue;
                        queue.push(img_path.clone());
                        group.loading_image = None;
                        group.current_image.clone_from(&img_path);
                    }
                }
                self.current_img = img_path;
            }
        }
    }

    /// Update wallpaper by going down 1 index through the cached image paths.
    /// Expiry timer reset even if already at the first cached image.
    ///
    /// # Detour Semantics
    ///
    /// If the current image was set via `wpaperctl set`, this returns to the
    /// image that was showing before the set (doesn't decrement the index).
    /// This implements "detour" behavior: forced images are temporary diversions
    /// that don't affect navigation history.
    ///
    /// # Implementation Note
    ///
    /// When returning from a forced image, we reuse the `reload` flag to trigger
    /// a redisplay of the current index position. This is a non-standard use of
    /// `reload` (which normally means "refresh current image from disk"), but it
    /// achieves the desired behavior without adding another flag. The `reload`
    /// flag causes `get_image_from_path` to return the current index's image
    /// even if it matches `current_img`.
    pub fn previous_image(&mut self) {
        if self.was_last_forced {
            // DETOUR RETURN: User called `previous` after a forced image.
            // Instead of going back in history, return to the image that was
            // showing before the forced image. We reuse `reload` to trigger
            // `get_image_from_path` to re-fetch the current index position.
            self.reload = true;
            self.was_last_forced = false;
        } else {
            self.actions.push_back(ImagePickerAction::Previous);
        }
    }

    /// A forced image could not be loaded: the displayed image, and whether it is itself a
    /// detour, are unchanged.
    pub fn forced_load_failed(&mut self) {
        self.was_last_forced = self.forced_fallback;
    }

    /// Update wallpaper by going up 1 index through the cached image paths
    pub fn next_image(&mut self) {
        // Clear forced flag - next continues normal navigation
        self.was_last_forced = false;
        self.actions.push_back(ImagePickerAction::Next);
    }

    pub fn current_image(&self) -> PathBuf {
        self.current_img.clone()
    }

    /// Apply a reloaded output config to the chooser.
    ///
    /// A changed pool always resets future navigation, so next/previous cannot return to images
    /// of the old pool. The current image is replaced separately: only a drawn, non-forced image
    /// outside the new pool. Returns true when the current image must be replaced now.
    pub fn update_source(
        &mut self,
        old: &WallpaperInfo,
        new: &WallpaperInfo,
        wl_surface: &WlSurface,
        wallpaper_groups: &Rc<RefCell<WallpaperGroups>>,
    ) -> bool {
        let pool_changed = pool_changed(old, new);
        let replace = pool_changed
            && !self.was_last_forced
            && !self.current_img.as_os_str().is_empty()
            && !pool_contains(&self.filelist_cache, new, &self.current_img);
        self.update_sorting(new, wl_surface, pool_changed, wallpaper_groups);
        replace
    }

    /// Return true if the path changed
    fn update_sorting(
        &mut self,
        wallpaper_info: &WallpaperInfo,
        wl_surface: &WlSurface,
        path_changed: bool,
        wallpaper_groups: &Rc<RefCell<WallpaperGroups>>,
    ) {
        if let Some(new_sorting) = wallpaper_info.sorting {
            match (&mut self.sorting, new_sorting) {
                // If the the sorting stayed the same, do nothing
                (ImagePickerSorting::Ascending(_), Sorting::Ascending)
                | (ImagePickerSorting::Descending(_), Sorting::Descending)
                | (ImagePickerSorting::Random(_), Sorting::Random)
                    if !path_changed => {}
                (_, Sorting::Ascending) if path_changed => {
                    let files_len = if wallpaper_info.path.is_dir() {
                        self.filelist_cache
                            .borrow()
                            .get(
                                &wallpaper_info.path,
                                wallpaper_info.recursive.unwrap_or_default(),
                            )
                            .len()
                    } else {
                        0
                    };
                    self.sorting = ImagePickerSorting::new_ascending(files_len);
                }
                (_, Sorting::Descending) if path_changed => {
                    self.sorting = ImagePickerSorting::new_descending();
                }
                (_, Sorting::Ascending | Sorting::Descending) => {
                    let index = self.get_current_index();
                    self.sorting = match new_sorting {
                        Sorting::Random | Sorting::GroupedRandom { .. } => unreachable!(),
                        Sorting::Ascending => ImagePickerSorting::Ascending(index),
                        Sorting::Descending => ImagePickerSorting::Descending(index),
                    };
                }
                // The pool has changed, start a new history that keeps only a current image
                // that is still in the pool, otherwise we reuse the current drawn_images
                (_, Sorting::Random) if path_changed => {
                    let mut queue = Queue::with_capacity(wallpaper_info.drawn_images_queue_size);
                    if pool_contains(&self.filelist_cache, wallpaper_info, &self.current_img) {
                        queue.push(self.current_image());
                    }
                    self.sorting = ImagePickerSorting::Random(queue);
                }
                (_, Sorting::Random) => {
                    // if the path was not changed, use the current image as the first image of
                    // the drawn_images
                    let mut queue = Queue::with_capacity(wallpaper_info.drawn_images_queue_size);
                    queue.push(self.current_image());
                    self.sorting = ImagePickerSorting::Random(queue);
                }
                // If the group is the same
                (
                    ImagePickerSorting::GroupedRandom(grouped_random),
                    Sorting::GroupedRandom { group },
                ) if grouped_random.group.borrow().group == group => {
                    if path_changed {
                        // Reset the shared history in place: replacing this GroupedRandom would
                        // drop it, and its Drop removes this surface from the registered group.
                        let mut shared = grouped_random.group.borrow_mut();
                        let mut queue =
                            Queue::with_capacity(wallpaper_info.drawn_images_queue_size);
                        if pool_contains(
                            &self.filelist_cache,
                            wallpaper_info,
                            &shared.current_image,
                        ) {
                            queue.push(shared.current_image.clone());
                        }
                        shared.queue = queue;
                    }
                }
                (_, Sorting::GroupedRandom { group }) => {
                    let grouped_random = GroupedRandom::new(
                        wallpaper_groups.clone(),
                        group,
                        wl_surface,
                        wallpaper_info.drawn_images_queue_size,
                    );

                    let mut group = grouped_random.group.borrow_mut();
                    // If there are no other surfaces, we must reuse the current wallpaper. After a
                    // pool change it only enters the history while it is still in the pool.
                    if group.surfaces.len() == 1 {
                        group.current_image = self.current_img.clone();
                        if !path_changed
                            || pool_contains(
                                &self.filelist_cache,
                                wallpaper_info,
                                &self.current_img,
                            )
                        {
                            group.queue.push(self.current_img.clone());
                        }
                    }

                    drop(group);
                    self.sorting = ImagePickerSorting::GroupedRandom(grouped_random);
                }
            }
        } else {
            self.sorting = ImagePickerSorting::new_random(wallpaper_info.drawn_images_queue_size);
        }
    }

    fn get_current_index(&mut self) -> usize {
        match &self.sorting {
            ImagePickerSorting::Random(_) | ImagePickerSorting::GroupedRandom(_) => unreachable!(),
            ImagePickerSorting::Ascending(index) | ImagePickerSorting::Descending(index) => *index,
        }
    }

    pub fn update_queue_size(&mut self, drawn_images_queue_size: usize) {
        match &mut self.sorting {
            ImagePickerSorting::Random(queue) => {
                queue.resize(drawn_images_queue_size);
            }
            ImagePickerSorting::Ascending(_) | ImagePickerSorting::Descending(_) => {}
            ImagePickerSorting::GroupedRandom(group) => {
                group
                    .group
                    .borrow_mut()
                    .queue
                    .resize(drawn_images_queue_size);
            }
        }
    }

    #[inline]
    pub fn reload(&mut self) {
        self.reload = true;
    }

    #[inline]
    pub fn reloaded(&mut self) {
        self.reload = false;
    }

    #[inline]
    pub fn is_reloading(&self) -> bool {
        self.reload
    }

    pub fn handle_grouped_sorting(&self, qh: &QueueHandle<Wpaperd>) {
        if let ImagePickerSorting::GroupedRandom(grouped_random) = &self.sorting {
            grouped_random.group.borrow().queue_all_surfaces(qh);
        }
    }

    pub fn clear_first_action(&mut self) {
        self.actions.pop_front();
    }
}

/// The pool is the configured path walked with its effective recursion.
fn pool_changed(old: &WallpaperInfo, new: &WallpaperInfo) -> bool {
    old.path != new.path
        || (new.path.is_dir()
            && old.recursive.unwrap_or_default() != new.recursive.unwrap_or_default())
}

fn pool_contains(
    filelist_cache: &RefCell<FilelistCache>,
    wallpaper_info: &WallpaperInfo,
    image: &Path,
) -> bool {
    if wallpaper_info.path.is_dir() {
        filelist_cache
            .borrow()
            .get(
                &wallpaper_info.path,
                wallpaper_info.recursive.unwrap_or_default(),
            )
            .iter()
            .any(|path| path == image)
    } else {
        wallpaper_info.path == image
    }
}

fn next_random_image(current_image: &Path, queue: &mut Queue, files: &[PathBuf]) -> PathBuf {
    // Use the next images in the queue, if any
    let wrap = files.len() <= queue.len();
    // If there is only one image just return it
    if files.len() == 1 {
        return files[0].to_path_buf();
    }

    while let Some(next) = queue.next(wrap) {
        if next.exists() {
            return next.to_path_buf();
        }
    }

    // Otherwise pick a new random image that has not been drawn before
    // Try 5 times, then get a random image. We do this because it might happen
    // that the queue is bigger than the amount of available wallpapers
    let mut tries = 5;
    loop {
        let index = fastrand::usize(..files.len());
        // search for an image that has not been drawn yet
        // fail after 5 tries
        if !queue.contains(&files[index]) {
            break files[index].to_path_buf();
        }

        // We have already tried a bunch of times
        // We still need a new image, get the first one that is different than
        // the current one. We also know that there is more than one image
        if tries == 0 {
            break loop {
                let index = fastrand::usize(..files.len());
                if files[index] != current_image {
                    break files[index].to_path_buf();
                }
            };
        }

        tries -= 1;
    }
}

fn get_previous_image_for_random(current_image: &Path, queue: &mut Queue, wrap: bool) -> PathBuf {
    while let Some(prev) = queue.previous(wrap) {
        if prev.exists() {
            return prev.to_path_buf();
        }
    }

    // We didn't find any suitable image, reset to the last working one
    queue.set_current_to(current_image);
    current_image.to_path_buf()
}

#[cfg(test)]
mod tests {
    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;

    #[test]
    fn test_push() {
        let mut queue = Queue::with_capacity(2);
        queue.push(PathBuf::from("mypath"));
        queue.push(PathBuf::from("mypath2"));
        assert_eq!(Path::new("mypath2"), queue.current());
        assert_eq!(Some(Path::new("mypath")), queue.previous(false));
        assert_eq!(Path::new("mypath"), queue.current());

        assert_eq!(None, queue.previous(false));
    }

    #[test]
    fn test_resize() {
        let mut queue = Queue::with_capacity(5);
        queue.push(PathBuf::from("mypath"));
        queue.push(PathBuf::from("mypath2"));
        queue.push(PathBuf::from("mypath3"));
        queue.push(PathBuf::from("mypath4"));
        queue.push(PathBuf::from("mypath5"));
        assert_eq!(queue.buffer.len(), 5);
        assert_eq!(Path::new("mypath5"), queue.current());
        assert_eq!(Some(Path::new("mypath4")), queue.previous(false));

        // Test that the current index works when it's inside the resizing range
        queue.resize(2);
        assert_eq!(Path::new("mypath4"), queue.current());
        assert_eq!(None, queue.previous(false));
        assert_eq!(Some(Path::new("mypath5")), queue.next(false));
    }

    #[test]
    fn test_resize2() {
        let mut queue = Queue::with_capacity(5);
        queue.push(PathBuf::from("mypath"));
        queue.push(PathBuf::from("mypath2"));
        queue.push(PathBuf::from("mypath3"));
        queue.push(PathBuf::from("mypath4"));
        queue.push(PathBuf::from("mypath5"));
        queue.push(PathBuf::from("mypath6"));
        queue.push(PathBuf::from("mypath7"));
        queue.push(PathBuf::from("mypath8"));
        assert_eq!(queue.buffer.len(), 5);
        assert_eq!(Path::new("mypath8"), queue.current());
        assert_eq!(Some(Path::new("mypath7")), queue.previous(false));
        assert_eq!(Some(Path::new("mypath6")), queue.previous(false));
        assert_eq!(Some(Path::new("mypath5")), queue.previous(false));

        // Test that the current item point to the first item available
        queue.resize(2);
        assert_eq!(Path::new("mypath7"), queue.current());
        assert_eq!(Some(Path::new("mypath8")), queue.next(false));
        assert_eq!(None, queue.next(false));
    }

    // =======================================================
    // Tests for Queue interaction with "set" behavior
    // =======================================================

    #[test]
    fn test_queue_set_then_previous_navigates_back() {
        let mut queue = Queue::with_capacity(5);

        // Build up some history
        queue.push(PathBuf::from("/image1.png"));
        queue.push(PathBuf::from("/image2.png"));
        queue.push(PathBuf::from("/image3.png"));

        // User sets a manual wallpaper (push simulates adding to history)
        queue.push(PathBuf::from("/manual.png"));
        assert_eq!(queue.current(), Path::new("/manual.png"));

        // User immediately does `previous` - should go to image3
        let prev = queue.previous(false);
        assert_eq!(prev, Some(Path::new("/image3.png")));
    }

    #[test]
    fn test_queue_set_existing_image_is_noop() {
        let mut queue = Queue::with_capacity(5);

        queue.push(PathBuf::from("/image1.png"));
        queue.push(PathBuf::from("/image2.png"));

        // User sets the same image that's already current
        // Queue::push already handles this - duplicates are ignored
        queue.push(PathBuf::from("/image2.png"));

        // Still at image2, buffer should not have grown
        assert_eq!(queue.current(), Path::new("/image2.png"));
        assert_eq!(queue.buffer.len(), 2);

        // Previous goes to image1
        let prev = queue.previous(false);
        assert_eq!(prev, Some(Path::new("/image1.png")));

        // No more previous (we're at the start)
        assert_eq!(queue.previous(false), None);
    }

    #[test]
    fn test_queue_set_current_to_moves_cursor() {
        let mut queue = Queue::with_capacity(5);

        queue.push(PathBuf::from("/image1.png"));
        queue.push(PathBuf::from("/image2.png"));
        queue.push(PathBuf::from("/image3.png"));
        assert_eq!(queue.current(), Path::new("/image3.png"));

        // set_current_to moves cursor to existing image in history
        queue.set_current_to(Path::new("/image1.png"));
        assert_eq!(queue.current(), Path::new("/image1.png"));

        // Next should go to image2
        let next = queue.next(false);
        assert_eq!(next, Some(Path::new("/image2.png")));
    }

    #[test]
    fn test_queue_set_current_to_nonexistent_is_noop() {
        let mut queue = Queue::with_capacity(5);

        queue.push(PathBuf::from("/image1.png"));
        queue.push(PathBuf::from("/image2.png"));
        assert_eq!(queue.current(), Path::new("/image2.png"));

        // Try to set to a path not in history - set_current_to is a no-op
        queue.set_current_to(Path::new("/nonexistent.png"));

        // Current should still be image2
        assert_eq!(queue.current(), Path::new("/image2.png"));
    }

    // =======================================================
    // Tests for ImageResult enum
    // =======================================================

    #[test]
    fn test_image_result_path_returns_correct_path() {
        let forced = ImageResult::Forced(PathBuf::from("/forced.png"));
        assert_eq!(forced.path(), Path::new("/forced.png"));

        let from_list = ImageResult::FromList {
            path: PathBuf::from("/list.png"),
            index: None,
        };
        assert_eq!(from_list.path(), Path::new("/list.png"));
    }

    #[test]
    fn test_image_result_forced_variant() {
        let result = ImageResult::Forced(PathBuf::from("/test.png"));
        assert!(matches!(result, ImageResult::Forced(_)));
    }

    #[test]
    fn test_image_result_from_list_variant() {
        let result = ImageResult::FromList {
            path: PathBuf::from("/test.png"),
            index: None,
        };
        match result {
            ImageResult::FromList { path, index } => {
                assert_eq!(path, PathBuf::from("/test.png"));
                assert_eq!(index, None);
            }
            _ => panic!("Expected FromList variant"),
        }
    }

    // =======================================================
    // Tests for ImagePicker forced image state transitions
    // =======================================================
    //
    // Note: ImagePicker requires complex dependencies (WlSurface, FilelistCache, etc.)
    // that are difficult to construct in unit tests. These tests focus on the
    // state machine logic by testing the individual state transitions.
    //
    // Integration testing of the full set->previous/next flow should be done
    // via manual testing with the actual daemon.

    /// Helper struct to test ImagePicker state transitions without full dependencies
    struct ForcedImageState {
        was_last_forced: bool,
        reload: bool,
        action: Option<ImagePickerAction>,
    }

    impl ForcedImageState {
        fn new() -> Self {
            Self {
                was_last_forced: false,
                reload: false,
                action: None,
            }
        }

        /// Simulates the state change when a forced image is loaded
        fn simulate_forced_image_loaded(&mut self) {
            self.was_last_forced = true;
        }

        /// Simulates previous_image() behavior
        fn previous_image(&mut self) {
            if self.was_last_forced {
                // DETOUR RETURN: reload current index instead of going back
                self.reload = true;
                self.was_last_forced = false;
            } else {
                self.action = Some(ImagePickerAction::Previous);
            }
        }

        /// Simulates next_image() behavior (just the state transition part)
        fn next_image(&mut self) {
            self.was_last_forced = false;
            self.action = Some(ImagePickerAction::Next);
        }
    }

    #[test]
    fn test_forced_image_previous_triggers_reload() {
        let mut state = ForcedImageState::new();

        // Simulate: user did `wpaperctl set`, image loaded
        state.simulate_forced_image_loaded();
        assert!(state.was_last_forced);
        assert!(!state.reload);

        // User calls `previous`
        state.previous_image();

        // Should trigger reload (return to pre-set image), NOT set Previous action
        assert!(state.reload);
        assert!(!state.was_last_forced);
        assert!(state.action.is_none());
    }

    #[test]
    fn test_forced_image_next_clears_flag_and_continues() {
        let mut state = ForcedImageState::new();

        // Simulate: user did `wpaperctl set`, image loaded
        state.simulate_forced_image_loaded();
        assert!(state.was_last_forced);

        // User calls `next`
        state.next_image();

        // Should clear forced flag and continue normal navigation
        assert!(!state.was_last_forced);
        assert!(matches!(state.action, Some(ImagePickerAction::Next)));
    }

    #[test]
    fn test_normal_previous_sets_action() {
        let mut state = ForcedImageState::new();

        // No forced image - normal state
        assert!(!state.was_last_forced);

        // User calls `previous`
        state.previous_image();

        // Should set Previous action, not reload
        assert!(!state.reload);
        assert!(matches!(state.action, Some(ImagePickerAction::Previous)));
    }

    #[test]
    fn test_detour_semantics_sequence() {
        let mut state = ForcedImageState::new();

        // 1. User is viewing normal image, calls next a few times
        state.next_image();
        assert!(matches!(state.action, Some(ImagePickerAction::Next)));
        state.action = None; // Reset for next operation

        // 2. User does `wpaperctl set /some/image.png`
        state.simulate_forced_image_loaded();
        assert!(state.was_last_forced);

        // 3. User calls `previous` - should return to pre-set image (reload)
        state.previous_image();
        assert!(state.reload);
        assert!(!state.was_last_forced);
        assert!(state.action.is_none()); // No Previous action - reload handles it

        // 4. Reset reload flag (simulating it was consumed)
        state.reload = false;

        // 5. Now user calls `previous` again - normal navigation
        state.previous_image();
        assert!(!state.reload);
        assert!(matches!(state.action, Some(ImagePickerAction::Previous)));
    }
}

/// Reload regressions for Violet-Void/wpaperd#2: a changed pool must not keep old navigation.
#[cfg(test)]
mod pool_tests {
    use std::{
        fs,
        os::unix::net::UnixStream,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use smithay_client_toolkit::reexports::client::{backend::Backend, Proxy};

    use super::*;

    /// A unique real directory tree, removed on drop.
    struct Tree(PathBuf);

    impl Tree {
        fn new(files: &[&str]) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!(
                "wpaperd-pool-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&root).unwrap();
            for file in files {
                let path = root.join(file);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, b"").unwrap();
            }
            Self(root)
        }

        fn join(&self, relative: &str) -> PathBuf {
            self.0.join(relative)
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Picker dependencies without a compositor. Inert surfaces all compare equal, so this
    /// covers a single output per wallpaper group.
    struct Harness {
        _backend: Backend,
        _peer: UnixStream,
        surface: WlSurface,
        groups: Rc<RefCell<WallpaperGroups>>,
        cache: Rc<RefCell<FilelistCache>>,
    }

    fn harness(pools: &[(PathBuf, Recursive)]) -> Harness {
        fastrand::seed(2);
        let (stream, peer) = UnixStream::pair().unwrap();
        let backend = Backend::connect(stream).unwrap();
        let surface = WlSurface::inert(backend.downgrade());
        Harness {
            _backend: backend,
            _peer: peer,
            surface,
            groups: Rc::new(RefCell::new(WallpaperGroups::new())),
            cache: Rc::new(RefCell::new(FilelistCache::populated(pools))),
        }
    }

    fn info(
        path: PathBuf,
        sorting: Option<Sorting>,
        recursive: Option<Recursive>,
    ) -> WallpaperInfo {
        WallpaperInfo {
            path,
            sorting,
            recursive,
            ..Default::default()
        }
    }

    fn picker(h: &Harness, info: &WallpaperInfo) -> ImagePicker {
        ImagePicker::new(info, &h.surface, h.cache.clone(), h.groups.clone())
    }

    fn pool(h: &Harness, info: &WallpaperInfo) -> Vec<PathBuf> {
        if info.path.is_dir() {
            h.cache
                .borrow()
                .get(&info.path, info.recursive.unwrap_or_default())
                .to_vec()
        } else {
            vec![info.path.clone()]
        }
    }

    /// One `Surface::load_wallpaper` pass with an immediately loaded image.
    fn draw(picker: &mut ImagePicker, info: &WallpaperInfo) -> PathBuf {
        match picker.get_image_from_path(&info.path, &info.recursive) {
            Some(_) if picker.is_reloading() => picker.reloaded(),
            Some(result) => picker.update_current_image(result),
            None => picker.clear_first_action(),
        }
        picker.current_image()
    }

    /// The picker half of `Surface::update_wallpaper_info`.
    fn reload(
        h: &Harness,
        picker: &mut ImagePicker,
        old: &WallpaperInfo,
        new: &WallpaperInfo,
    ) -> bool {
        let replace = picker.update_source(old, new, &h.surface, &h.groups);
        if replace {
            picker.next_image();
            draw(picker, new);
        }
        replace
    }

    /// Mixed previous/next navigation after a reload.
    fn walk(picker: &mut ImagePicker, info: &WallpaperInfo) -> Vec<PathBuf> {
        let mut seen = Vec::new();
        for previous in [true, true, false, false, false, true, true] {
            if previous {
                picker.previous_image();
            } else {
                picker.next_image();
            }
            seen.push(draw(picker, info));
        }
        seen
    }

    fn assert_within(seen: &[PathBuf], pool: &[PathBuf]) {
        for path in seen {
            assert!(pool.contains(path), "stale image {path:?} outside {pool:?}");
        }
    }

    /// Random history as left by earlier draws, ending on `current`.
    fn seed_history(picker: &mut ImagePicker, history: &[PathBuf], current: &Path) {
        let ImagePickerSorting::Random(queue) = &mut picker.sorting else {
            panic!("expected random sorting");
        };
        for path in history {
            queue.push(path.clone());
        }
        queue.set_current_to(current);
        picker.current_img = current.to_path_buf();
        picker.actions.clear();
    }

    fn build_history(picker: &mut ImagePicker, info: &WallpaperInfo) {
        draw(picker, info);
        for _ in 0..3 {
            picker.next_image();
            draw(picker, info);
        }
        // Leave the cursor inside the history so `next` can replay it too.
        picker.previous_image();
        draw(picker, info);
    }

    #[test]
    fn pool_change_uses_path_and_effective_recursion() {
        let root = Tree::new(&["a.png", "sub/b.png"]);
        let file = root.join("a.png");
        let dir = |path: &Path, recursive| info(path.to_path_buf(), None, recursive);
        let same = [
            (dir(&root.0, None), dir(&root.0, None)),
            (dir(&root.0, None), dir(&root.0, Some(Recursive::On))),
            (dir(&root.0, Some(Recursive::On)), dir(&root.0, None)),
            // Recursion does not apply to a single file.
            (dir(&file, None), dir(&file, Some(Recursive::Off))),
        ];
        for (old, new) in &same {
            assert!(!pool_changed(old, new), "{old:?} -> {new:?}");
        }
        let changed = [
            (dir(&root.0, None), dir(&root.0, Some(Recursive::Off))),
            (dir(&root.0, Some(Recursive::Off)), dir(&root.0, None)),
            (
                dir(&root.0, Some(Recursive::On)),
                dir(&root.0, Some(Recursive::Off)),
            ),
            (dir(&root.0, None), dir(&root.join("sub"), None)),
            (dir(&root.join("sub"), None), dir(&root.0, None)),
            (dir(&root.0, None), dir(&file, None)),
            (dir(&file, None), dir(&root.0, None)),
        ];
        for (old, new) in &changed {
            assert!(pool_changed(old, new), "{old:?} -> {new:?}");
        }
    }

    #[test]
    fn disjoint_pool_change_adopts_new_pool_without_stale_navigation() {
        let a = Tree::new(&["a1.png", "a2.png", "a3.png", "a4.png"]);
        let b = Tree::new(&["b1.png", "b2.png"]);
        let h = harness(&[(a.0.clone(), Recursive::On), (b.0.clone(), Recursive::On)]);
        let old = info(a.0.clone(), Some(Sorting::Random), None);
        let new = info(b.0.clone(), Some(Sorting::Random), None);
        let mut picker = picker(&h, &old);
        build_history(&mut picker, &old);
        assert!(pool(&h, &old).contains(&picker.current_image()));

        reload(&h, &mut picker, &old, &new);

        let b_pool = pool(&h, &new);
        assert_within(&[picker.current_image()], &b_pool);
        assert_within(&walk(&mut picker, &new), &b_pool);
    }

    #[test]
    fn nested_narrower_pool_keeps_member_and_drops_outside_history() {
        let root = Tree::new(&["x.png", "sub/y.png", "sub/z.png"]);
        let sub = root.join("sub");
        let h = harness(&[
            (root.0.clone(), Recursive::On),
            (sub.clone(), Recursive::On),
        ]);
        let old = info(root.0.clone(), Some(Sorting::Random), None);
        let new = info(sub.clone(), Some(Sorting::Random), None);
        let mut picker = picker(&h, &old);
        let y = sub.join("y.png");
        seed_history(&mut picker, &[root.join("x.png"), y.clone()], &y);

        assert!(!reload(&h, &mut picker, &old, &new), "member image is kept");
        assert_eq!(picker.current_image(), y);
        assert_within(&walk(&mut picker, &new), &pool(&h, &new));
    }

    #[test]
    fn nested_narrower_pool_replaces_non_member() {
        let root = Tree::new(&["x.png", "sub/y.png", "sub/z.png"]);
        let sub = root.join("sub");
        let h = harness(&[
            (root.0.clone(), Recursive::On),
            (sub.clone(), Recursive::On),
        ]);
        let old = info(root.0.clone(), Some(Sorting::Random), None);
        let new = info(sub.clone(), Some(Sorting::Random), None);
        let mut picker = picker(&h, &old);
        let x = root.join("x.png");
        seed_history(&mut picker, &[sub.join("y.png"), x.clone()], &x);

        reload(&h, &mut picker, &old, &new);

        let sub_pool = pool(&h, &new);
        assert_within(&[picker.current_image()], &sub_pool);
        assert_within(&walk(&mut picker, &new), &sub_pool);
    }

    #[test]
    fn nested_wider_pool_follows_effective_recursion() {
        let root = Tree::new(&["x.png", "w.png", "sub/y.png"]);
        let sub = root.join("sub");
        let y = sub.join("y.png");
        let h = harness(&[
            (root.0.clone(), Recursive::On),
            (root.0.clone(), Recursive::Off),
            (sub.clone(), Recursive::On),
        ]);
        let old = info(sub.clone(), Some(Sorting::Random), None);

        let wide = info(root.0.clone(), Some(Sorting::Random), None);
        let mut kept = picker(&h, &old);
        seed_history(&mut kept, std::slice::from_ref(&y), &y);
        assert!(
            !reload(&h, &mut kept, &old, &wide),
            "recursive parent holds y"
        );
        assert_eq!(kept.current_image(), y);

        let shallow = info(root.0.clone(), Some(Sorting::Random), Some(Recursive::Off));
        let mut replaced = picker(&h, &old);
        seed_history(&mut replaced, std::slice::from_ref(&y), &y);
        reload(&h, &mut replaced, &old, &shallow);
        let shallow_pool = pool(&h, &shallow);
        assert_within(&[replaced.current_image()], &shallow_pool);
        assert_within(&walk(&mut replaced, &shallow), &shallow_pool);
    }

    #[test]
    fn recursion_only_change_adopts_shallow_pool() {
        let root = Tree::new(&["top1.png", "top2.png", "sub/n1.png", "sub/n2.png"]);
        let h = harness(&[
            (root.0.clone(), Recursive::On),
            (root.0.clone(), Recursive::Off),
        ]);
        // `None` is effectively `Recursive::On`.
        let old = info(root.0.clone(), Some(Sorting::Random), None);
        let new = info(root.0.clone(), Some(Sorting::Random), Some(Recursive::Off));
        let history = [
            root.join("top1.png"),
            root.join("sub/n1.png"),
            root.join("top2.png"),
            root.join("sub/n2.png"),
        ];
        let shallow = pool(&h, &new);

        let mut nested = picker(&h, &old);
        seed_history(&mut nested, &history, &history[3]);
        reload(&h, &mut nested, &old, &new);
        assert_within(&[nested.current_image()], &shallow);
        assert_within(&walk(&mut nested, &new), &shallow);

        let mut top = picker(&h, &old);
        seed_history(&mut top, &history, &history[2]);
        assert!(!reload(&h, &mut top, &old, &new), "top-level image is kept");
        assert_eq!(top.current_image(), history[2]);
        assert_within(&walk(&mut top, &new), &shallow);
    }

    #[test]
    fn explicit_default_recursion_keeps_history() {
        let root = Tree::new(&["a.png", "b.png", "c.png"]);
        let h = harness(&[(root.0.clone(), Recursive::On)]);
        let old = info(root.0.clone(), Some(Sorting::Random), None);
        let new = info(root.0.clone(), Some(Sorting::Random), Some(Recursive::On));
        let mut picker = picker(&h, &old);
        let history = [root.join("a.png"), root.join("b.png")];
        seed_history(&mut picker, &history, &history[1]);

        assert!(!reload(&h, &mut picker, &old, &new));
        picker.previous_image();
        assert_eq!(draw(&mut picker, &new), history[0], "history is unchanged");
    }

    #[test]
    fn directory_to_file_and_back_replaces_current() {
        let a = Tree::new(&["a1.png", "a2.png", "a3.png"]);
        let single = Tree::new(&["only.png"]);
        let b = Tree::new(&["b1.png", "b2.png"]);
        let file = single.join("only.png");
        let h = harness(&[(a.0.clone(), Recursive::On), (b.0.clone(), Recursive::On)]);
        let dir_a = info(a.0.clone(), Some(Sorting::Random), None);
        // Config rejects sorting for a file path.
        let to_file = info(file.clone(), None, None);
        let dir_b = info(b.0.clone(), Some(Sorting::Random), None);
        let mut picker = picker(&h, &dir_a);
        build_history(&mut picker, &dir_a);

        reload(&h, &mut picker, &dir_a, &to_file);
        assert_eq!(picker.current_image(), file);
        assert_within(&walk(&mut picker, &to_file), std::slice::from_ref(&file));

        reload(&h, &mut picker, &to_file, &dir_b);
        let b_pool = pool(&h, &dir_b);
        assert_within(&[picker.current_image()], &b_pool);
        assert_within(&walk(&mut picker, &dir_b), &b_pool);
    }

    #[test]
    fn ordered_sorting_adopts_new_pool() {
        for sorting in [Sorting::Ascending, Sorting::Descending] {
            let a = Tree::new(&["a1.png", "a2.png", "a3.png"]);
            let b = Tree::new(&["b1.png", "b2.png", "b3.png"]);
            let h = harness(&[(a.0.clone(), Recursive::On), (b.0.clone(), Recursive::On)]);
            let old = info(a.0.clone(), Some(sorting), None);
            let new = info(b.0.clone(), Some(sorting), None);
            let mut picker = picker(&h, &old);
            build_history(&mut picker, &old);

            reload(&h, &mut picker, &old, &new);

            let b_pool = pool(&h, &new);
            assert_within(&[picker.current_image()], &b_pool);
            assert_within(&walk(&mut picker, &new), &b_pool);
        }
    }

    #[test]
    fn sorting_and_pool_change_together() {
        let a = Tree::new(&["a1.png", "a2.png", "a3.png"]);
        let b = Tree::new(&["b1.png", "b2.png", "b3.png"]);
        let h = harness(&[(a.0.clone(), Recursive::On), (b.0.clone(), Recursive::On)]);
        let old = info(a.0.clone(), Some(Sorting::Random), None);
        let new = info(b.0.clone(), Some(Sorting::Ascending), None);
        let mut picker = picker(&h, &old);
        build_history(&mut picker, &old);

        reload(&h, &mut picker, &old, &new);

        let b_pool = pool(&h, &new);
        assert_within(&[picker.current_image()], &b_pool);
        assert_within(&walk(&mut picker, &new), &b_pool);
    }

    #[test]
    fn grouped_pool_change_resets_shared_history_in_place() {
        let a = Tree::new(&["a1.png", "a2.png", "a3.png", "a4.png"]);
        let b = Tree::new(&["b1.png", "b2.png"]);
        let h = harness(&[(a.0.clone(), Recursive::On), (b.0.clone(), Recursive::On)]);
        let grouped = Some(Sorting::GroupedRandom { group: 3 });
        let old = info(a.0.clone(), grouped, None);
        let new = info(b.0.clone(), grouped, None);
        let mut picker = picker(&h, &old);
        build_history(&mut picker, &old);

        reload(&h, &mut picker, &old, &new);

        let ImagePickerSorting::GroupedRandom(grouped) = &picker.sorting else {
            panic!("expected grouped sorting");
        };
        let registered = h.groups.borrow_mut().get_or_insert(3, &h.surface, 10);
        assert!(
            Rc::ptr_eq(&grouped.group, &registered),
            "group stays registered"
        );
        assert_eq!(registered.borrow().surfaces.len(), 1);
        let b_pool = pool(&h, &new);
        assert_within(&[picker.current_image()], &b_pool);
        assert_within(&walk(&mut picker, &new), &b_pool);
    }

    #[test]
    fn pool_change_into_new_group_seeds_it_from_this_output() {
        let root = Tree::new(&["a1.png", "a2.png", "sub/b1.png", "sub/b2.png"]);
        let sub = root.join("sub");
        let h = harness(&[
            (root.0.clone(), Recursive::On),
            (sub.clone(), Recursive::On),
        ]);
        let old = info(root.0.clone(), Some(Sorting::Random), None);
        let new = info(sub.clone(), Some(Sorting::GroupedRandom { group: 7 }), None);
        let sub_pool = pool(&h, &new);
        let (a1, b1) = (root.join("a1.png"), sub.join("b1.png"));

        for (current, replaced) in [(a1.clone(), true), (b1.clone(), false)] {
            let mut picker = picker(&h, &old);
            seed_history(&mut picker, &[a1.clone(), b1.clone()], &current);

            assert_eq!(reload(&h, &mut picker, &old, &new), replaced);
            if !replaced {
                assert_eq!(picker.current_image(), b1);
            }
            let ImagePickerSorting::GroupedRandom(grouped) = &picker.sorting else {
                panic!("expected grouped sorting");
            };
            assert_eq!(grouped.group.borrow().current_image, picker.current_image());
            assert_within(&[picker.current_image()], &sub_pool);
            assert_within(&walk(&mut picker, &new), &sub_pool);
        }
    }

    #[test]
    fn forced_detour_survives_reload_and_navigation_uses_new_pool() {
        let a = Tree::new(&["a1.png", "a2.png", "a3.png", "a4.png"]);
        let b = Tree::new(&["b1.png", "b2.png"]);
        let forced = Tree::new(&["forced.png"]);
        let forced = forced.join("forced.png");
        let h = harness(&[(a.0.clone(), Recursive::On), (b.0.clone(), Recursive::On)]);
        let old = info(a.0.clone(), Some(Sorting::Random), None);
        let new = info(b.0.clone(), Some(Sorting::Random), None);
        let mut picker = picker(&h, &old);
        build_history(&mut picker, &old);
        picker.set_image(forced.clone());
        assert_eq!(draw(&mut picker, &old), forced);
        // Another output's decode runs an idle load pass on this one.
        assert_eq!(draw(&mut picker, &old), forced);

        assert!(!reload(&h, &mut picker, &old, &new), "explicit set is kept");
        assert_eq!(picker.current_image(), forced);

        let mut allowed = pool(&h, &new);
        allowed.push(forced.clone());
        // Detour return redisplays the current position, which is no longer in A.
        picker.previous_image();
        assert_within(&[draw(&mut picker, &new)], &allowed);
        picker.next_image();
        assert_within(&[draw(&mut picker, &new)], &pool(&h, &new));
        assert_within(&walk(&mut picker, &new), &pool(&h, &new));
    }

    #[test]
    fn failed_set_does_not_block_replacement() {
        let a = Tree::new(&["a1.png", "a2.png", "a3.png"]);
        let b = Tree::new(&["b1.png", "b2.png"]);
        let h = harness(&[(a.0.clone(), Recursive::On), (b.0.clone(), Recursive::On)]);
        let old = info(a.0.clone(), Some(Sorting::Random), None);
        let new = info(b.0.clone(), Some(Sorting::Random), None);
        let mut picker = picker(&h, &old);
        build_history(&mut picker, &old);
        let shown = picker.current_image();

        // `Surface::load_wallpaper` gets an undecodable forced image and retries the list.
        picker.set_image(a.join("broken.png"));
        let forced = picker.get_image_from_path(&old.path, &old.recursive);
        assert!(matches!(forced, Some(ImageResult::Forced(_))));
        picker.forced_load_failed();
        assert_eq!(draw(&mut picker, &old), shown);

        assert!(
            reload(&h, &mut picker, &old, &new),
            "old-pool image is replaced"
        );
        assert_within(&[picker.current_image()], &pool(&h, &new));
    }

    #[test]
    fn failed_set_over_a_set_keeps_the_first_detour() {
        let a = Tree::new(&["a1.png", "a2.png", "a3.png"]);
        let b = Tree::new(&["b1.png", "b2.png"]);
        let forced = Tree::new(&["good.png"]);
        let good = forced.join("good.png");
        let h = harness(&[(a.0.clone(), Recursive::On), (b.0.clone(), Recursive::On)]);
        let old = info(a.0.clone(), Some(Sorting::Random), None);
        let new = info(b.0.clone(), Some(Sorting::Random), None);
        let mut picker = picker(&h, &old);
        build_history(&mut picker, &old);
        picker.set_image(good.clone());
        assert_eq!(draw(&mut picker, &old), good);

        picker.set_image(forced.join("broken.png"));
        let failed = picker.get_image_from_path(&old.path, &old.recursive);
        assert!(matches!(failed, Some(ImageResult::Forced(_))));
        picker.forced_load_failed();
        assert_eq!(draw(&mut picker, &old), good);

        assert!(!reload(&h, &mut picker, &old, &new), "first set is kept");
        assert_eq!(picker.current_image(), good);
        picker.previous_image();
        assert!(picker.is_reloading(), "previous is still a detour return");
    }

    #[test]
    fn reload_before_first_draw_does_not_double_advance() {
        let a = Tree::new(&["a1.png", "a2.png"]);
        let b = Tree::new(&["b1.png", "b2.png"]);
        let h = harness(&[(a.0.clone(), Recursive::On), (b.0.clone(), Recursive::On)]);
        let old = info(a.0.clone(), Some(Sorting::Random), None);
        let new = info(b.0.clone(), Some(Sorting::Random), None);
        let mut picker = picker(&h, &old);

        assert!(
            !reload(&h, &mut picker, &old, &new),
            "first draw is still pending"
        );
        assert_within(&[draw(&mut picker, &new)], &pool(&h, &new));
        assert!(picker.actions.is_empty());
    }
}
