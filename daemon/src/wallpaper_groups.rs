use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::{Rc, Weak},
};

use smithay_client_toolkit::reexports::client::{protocol::wl_surface::WlSurface, QueueHandle};

use crate::{image_picker::Queue, wpaperd::Wpaperd};

pub(crate) struct FrameScheduler {
    pending: RefCell<crate::surface::FrameRequests>,
    motion: crate::scene::SharedMotion,
    name: String,
}
impl FrameScheduler {
    pub(crate) fn new(motion: crate::scene::SharedMotion, name: String) -> Self {
        Self {
            pending: RefCell::new(Default::default()),
            motion,
            name,
        }
    }
    pub(crate) fn request(&self) -> bool {
        self.pending
            .borrow_mut()
            .request(crate::scene::owns_output(&self.motion, &self.name))
    }
    pub(crate) fn request_group(&self) -> bool {
        // Still-group invalidations cannot change an owned scene's visible content.
        if crate::scene::owns_output(&self.motion, &self.name) {
            return false;
        }
        self.request()
    }
    pub(crate) fn received(&self) {
        self.pending.borrow_mut().received();
    }
}
type FrameRegistry = Rc<RefCell<HashMap<WlSurface, Weak<FrameScheduler>>>>;

pub struct WallpaperGroup {
    pub group: u8,
    pub current_image: PathBuf,
    pub loading_image: Option<PathBuf>,
    pub surfaces: HashSet<WlSurface>,
    pub queue: Queue,
    frames: FrameRegistry,
}

impl WallpaperGroup {
    fn new(group: u8, queue_size: usize, frames: FrameRegistry) -> Self {
        Self {
            group,
            current_image: PathBuf::from(""),
            loading_image: None,
            surfaces: HashSet::new(),
            queue: Queue::with_capacity(queue_size),
            frames,
        }
    }

    pub fn queue_all_surfaces(&self, qh: &QueueHandle<Wpaperd>) {
        for surface in &self.surfaces {
            if let Some(scheduler) = self.frames.borrow().get(surface).and_then(Weak::upgrade) {
                if scheduler.request_group() {
                    surface.frame(qh, surface.clone());
                }
                surface.commit();
            }
        }
    }
}

pub struct WallpaperGroups {
    groups: HashMap<u8, Rc<RefCell<WallpaperGroup>>>,
    frames: FrameRegistry,
}

impl WallpaperGroups {
    pub fn new() -> Self {
        Self {
            groups: HashMap::new(),
            frames: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    pub(crate) fn register(&mut self, surface: &WlSurface, scheduler: &Rc<FrameScheduler>) {
        self.frames
            .borrow_mut()
            .insert(surface.clone(), Rc::downgrade(scheduler));
    }
    pub(crate) fn unregister(&mut self, surface: &WlSurface) {
        self.frames.borrow_mut().remove(surface);
    }

    pub fn get_or_insert(
        &mut self,
        group: u8,
        wl_surface: &WlSurface,
        queue_size: usize,
    ) -> Rc<RefCell<WallpaperGroup>> {
        self.groups.entry(group).or_insert_with(|| {
            Rc::new(RefCell::new(WallpaperGroup::new(
                group,
                queue_size,
                self.frames.clone(),
            )))
        });
        let wp_group = self.groups.get_mut(&group).unwrap();
        wp_group.borrow_mut().surfaces.insert(wl_surface.clone());
        wp_group.clone()
    }

    pub fn remove(&mut self, group: u8, wl_surface: &WlSurface) {
        let Some(wp_group) = self.groups.get(&group) else {
            return;
        };
        let mut wp_group = wp_group.borrow_mut();
        // if this wl_surface is the last one for this WallpaperGroup
        if wp_group.surfaces.len() == 1 {
            drop(wp_group);
            // Remove it from the WallpaperGroups
            self.groups.remove(&group);
        } else {
            wp_group.surfaces.remove(wl_surface);
        }
    }
}
