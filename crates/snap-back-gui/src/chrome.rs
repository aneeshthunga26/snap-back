//! Client-side caption controls inspired by T3's 40 px hidden-titlebar overlay.
//! Window actions are handed to the compositor; no privileged helper is involved.
use super::*;
use iced::widget::column;
use iced::{
    mouse,
    widget::{mouse_area, stack, svg, tooltip},
    window,
};

const HEIGHT: f32 = 40.0;
const EDGE: f32 = 6.0;

#[derive(Debug, Clone, Copy)]
pub(super) enum Action {
    Minimize,
    ToggleMaximize,
    Close,
    Drag,
    Resize(window::Direction),
}
#[derive(Debug, Clone)]
pub(super) enum Event {
    Action(Action),
    Ready(Option<window::Id>, Action),
    Observe(window::Id),
    Maximized(bool),
    Focused(bool),
    ThemeLoaded(super::desktop_theme::DesktopTheme),
}

pub(super) struct WindowChrome {
    id: Option<window::Id>,
    maximized: bool,
    focused: bool,
    minimize: svg::Handle,
    maximize: svg::Handle,
    restore: svg::Handle,
    close: svg::Handle,
    appearance: super::desktop_theme::Appearance,
    layout: super::desktop_theme::Layout,
}
impl Default for WindowChrome {
    fn default() -> Self {
        fn icon(body: &str) -> svg::Handle {
            svg::Handle::from_memory(format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 12 12" fill="none" stroke="#f8fafc" stroke-width="1">{body}</svg>"##).into_bytes())
        }
        Self {
            appearance: super::desktop_theme::Appearance::Adwaita,
            layout: super::desktop_theme::Layout::default(),
            id: None,
            maximized: false,
            focused: true,
            minimize: icon(r#"<path d="M1 6.5h10"/>"#),
            maximize: icon(r#"<rect x="1.5" y="1.5" width="9" height="9"/>"#),
            restore: icon(r#"<path d="M3.5 3.5v-2h7v7h-2M1.5 3.5h7v7h-7z"/>"#),
            close: icon(r#"<path d="m1.5 1.5 9 9m0-9-9 9"/>"#),
        }
    }
}
impl WindowChrome {
    pub fn load_theme() -> Task<Message> {
        Task::perform(super::desktop_theme::DesktopTheme::load(), |theme| {
            Message::Chrome(Event::ThemeLoaded(theme))
        })
    }
    pub fn has_leading_controls(&self) -> bool {
        !self.layout.left.is_empty()
    }
    pub fn corner_radius(&self) -> f32 {
        if self.maximized { 0.0 } else { 8.0 }
    }
    pub fn subscription() -> Subscription<Message> {
        iced::event::listen_with(|event, _, id| match event {
            iced::Event::Window(window::Event::Opened { .. } | window::Event::Resized(_)) => {
                Some(Message::Chrome(Event::Observe(id)))
            }
            iced::Event::Window(window::Event::Focused) => {
                Some(Message::Chrome(Event::Focused(true)))
            }
            iced::Event::Window(window::Event::Unfocused) => {
                Some(Message::Chrome(Event::Focused(false)))
            }
            _ => None,
        })
    }
    pub fn update(&mut self, event: Event) -> Task<Message> {
        match event {
            Event::ThemeLoaded(theme) => {
                if std::env::var_os("SNAP_BACK_DEBUG_CHROME").is_some() {
                    eprintln!(
                        "Caption theme: {} ({:?}), layout {:?}, icons {:?}",
                        theme.icon_theme, theme.appearance, theme.layout, theme.icons
                    );
                }
                self.appearance = theme.appearance;
                self.layout = theme.layout;
                for (handle, path) in [
                    &mut self.minimize,
                    &mut self.maximize,
                    &mut self.restore,
                    &mut self.close,
                ]
                .into_iter()
                .zip(theme.icons)
                {
                    if let Some(bytes) = path.and_then(|path| std::fs::read(path).ok()) {
                        *handle = svg::Handle::from_memory(bytes);
                    }
                }
                Task::none()
            }
            Event::Action(action) => {
                if let Some(id) = self.id {
                    Self::perform(id, action)
                } else {
                    window::oldest().map(move |id| Message::Chrome(Event::Ready(id, action)))
                }
            }
            Event::Ready(id, action) => {
                self.id = id;
                id.map(|id| Self::perform(id, action))
                    .unwrap_or_else(Task::none)
            }
            Event::Observe(id) => {
                self.id = Some(id);
                window::is_maximized(id).map(|yes| Message::Chrome(Event::Maximized(yes)))
            }
            Event::Maximized(yes) => {
                self.maximized = yes;
                Task::none()
            }
            Event::Focused(yes) => {
                self.focused = yes;
                self.id
                    .map(|id| {
                        window::is_maximized(id).map(|yes| Message::Chrome(Event::Maximized(yes)))
                    })
                    .unwrap_or_else(Task::none)
            }
        }
    }
    fn perform(id: window::Id, action: Action) -> Task<Message> {
        match action {
            Action::Minimize => window::minimize(id, true),
            Action::ToggleMaximize => window::toggle_maximize(id)
                .chain(window::is_maximized(id).map(|yes| Message::Chrome(Event::Maximized(yes)))),
            Action::Close => window::close(id),
            Action::Drag => window::drag(id),
            Action::Resize(direction) => window::drag_resize(id, direction),
        }
    }
    pub fn titlebar(&self) -> Element<'_, Message> {
        use super::desktop_theme::Appearance;
        let breeze = self.appearance == Appearance::Breeze;
        let group = |controls: &[super::desktop_theme::Control]| {
            row(controls.iter().map(|control| self.control(*control)))
                .spacing(if breeze { 0 } else { 3 })
                .align_y(Alignment::Center)
        };
        row![
            group(&self.layout.left),
            drag_region(Space::new().width(Fill).height(HEIGHT)),
            group(&self.layout.right),
        ]
        .padding([0, if breeze { 6 } else { 5 }])
        .align_y(Alignment::Center)
        .width(Fill)
        .height(HEIGHT)
        .into()
    }
    fn control(&self, control: super::desktop_theme::Control) -> Element<'_, Message> {
        use super::desktop_theme::{Appearance, Control};
        let (icon, label, action) = match control {
            Control::Minimize => (&self.minimize, "Minimize", Action::Minimize),
            Control::Maximize if self.maximized => {
                (&self.restore, "Restore", Action::ToggleMaximize)
            }
            Control::Maximize => (&self.maximize, "Maximize", Action::ToggleMaximize),
            Control::Close => (&self.close, "Close Snap Back", Action::Close),
        };
        let close = control == Control::Close;
        let breeze = self.appearance == Appearance::Breeze;
        let diameter = if breeze { 18.0 } else { 24.0 };
        let foreground = WHITE.scale_alpha(if self.focused { 1.0 } else { 0.55 });
        // Keep the SVG hit area equal to the circle so its hover filter changes
        // over the whole button. ScaleDown centers native-size icons within it;
        // ContentFit::None anchors them at the top-left in Iced 0.14.
        let glyph = svg(icon.clone())
            .width(diameter)
            .height(diameter)
            .content_fit(iced::ContentFit::ScaleDown)
            .style(move |_, status| svg::Style {
                color: Some(if breeze && status == svg::Status::Hovered {
                    BG
                } else {
                    foreground
                }),
            });
        let button = button(glyph)
            .width(diameter)
            .height(diameter)
            .padding(0)
            .on_press(Message::Chrome(Event::Action(action)))
            .style(move |_, state| {
                let active = matches!(state, button::Status::Hovered | button::Status::Pressed);
                let background = if breeze {
                    active.then(|| {
                        if close {
                            Color::from_rgb8(237, 87, 102)
                        } else {
                            foreground
                        }
                    })
                } else {
                    Some(foreground.scale_alpha(match state {
                        button::Status::Hovered => 0.15,
                        button::Status::Pressed => 0.30,
                        _ => 0.10,
                    }))
                };
                button::Style {
                    background: background.map(Into::into),
                    text_color: foreground,
                    border: Border::default().rounded(diameter / 2.0),
                    ..Default::default()
                }
            });
        container(
            tooltip(
                button,
                container(text(label).size(11))
                    .padding([5, 8])
                    .style(|_| container::Style {
                        background: Some(RAISED.into()),
                        border: Border::default().rounded(4),
                        ..Default::default()
                    }),
                tooltip::Position::Bottom,
            )
            .delay(Duration::from_millis(500)),
        )
        .padding(if breeze { 2 } else { 5 })
        .into()
    }
    pub fn content_pane<'a>(
        &self,
        content: impl Into<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        let radius = self.corner_radius();
        let outer = iced::border::Radius {
            top_right: if self.has_leading_controls() {
                0.0
            } else {
                radius
            },
            bottom_right: radius,
            ..Default::default()
        };
        let inner = iced::border::Radius {
            top_right: (outer.top_right - 1.0).max(0.0),
            bottom_right: (outer.bottom_right - 1.0).max(0.0),
            ..Default::default()
        };
        // The background forms three one-pixel edges. Zero left padding lets
        // the sidebar's existing border remain the only shared divider.
        container(
            container(content)
                .width(Fill)
                .height(Fill)
                .style(move |_| container::Style {
                    background: Some(BG.into()),
                    border: Border::default().rounded(inner),
                    ..Default::default()
                }),
        )
        .padding(iced::Padding {
            top: 1.0,
            right: 1.0,
            bottom: 1.0,
            left: 0.0,
        })
        .width(Fill)
        .height(Fill)
        .style(move |_| container::Style {
            background: Some(LINE.into()),
            border: Border::default().rounded(outer),
            ..Default::default()
        })
        .into()
    }

    pub fn frame<'a>(&self, content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
        let radius = self.corner_radius();
        let content: Element<'a, Message> = container(content)
            .width(Fill)
            .height(Fill)
            .style(move |_| container::Style {
                background: Some(BG.into()),
                border: Border::default().rounded(radius),
                ..Default::default()
            })
            .into();
        if self.maximized {
            return content;
        }
        // Borderless Wayland windows have no compositor resize border. These
        // narrow hit regions restore edge/corner resizing without covering UI.
        use window::Direction::*;
        let resize = column![
            row![
                resize_region(NorthWest, EDGE.into(), EDGE.into()),
                resize_region(North, Fill, EDGE.into()),
                resize_region(NorthEast, EDGE.into(), EDGE.into())
            ]
            .height(EDGE),
            row![
                resize_region(West, EDGE.into(), Fill),
                Space::new().width(Fill).height(Fill),
                resize_region(East, EDGE.into(), Fill)
            ]
            .height(Fill),
            row![
                resize_region(SouthWest, EDGE.into(), EDGE.into()),
                resize_region(South, Fill, EDGE.into()),
                resize_region(SouthEast, EDGE.into(), EDGE.into())
            ]
            .height(EDGE),
        ]
        .width(Fill)
        .height(Fill);
        stack![content, resize].width(Fill).height(Fill).into()
    }
}

pub(super) fn drag_region<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    mouse_area(content)
        .on_press(Message::Chrome(Event::Action(Action::Drag)))
        .on_double_click(Message::Chrome(Event::Action(Action::ToggleMaximize)))
        .into()
}
fn resize_region<'a>(
    direction: window::Direction,
    width: iced::Length,
    height: iced::Length,
) -> Element<'a, Message> {
    use window::Direction::*;
    let cursor = match direction {
        North | South => mouse::Interaction::ResizingVertically,
        East | West => mouse::Interaction::ResizingHorizontally,
        NorthWest | SouthEast => mouse::Interaction::ResizingDiagonallyDown,
        NorthEast | SouthWest => mouse::Interaction::ResizingDiagonallyUp,
    };
    mouse_area(Space::new().width(width).height(height))
        .on_press(Message::Chrome(Event::Action(Action::Resize(direction))))
        .interaction(cursor)
        .into()
}
