use super::*;
use composition::{ComposedCursor, LayoutIdentity, text_cells};
use std::collections::BTreeMap;
use zterm_core::terminal::{TerminalCell, TerminalStyle};

/// Client chrome before there is an authoritative Session surface.
pub(super) struct StartupProgress {
    target: String,
    observer: ProgressObserver,
    history: watch::Receiver<ProgressHistory>,
    cancelling: bool,
}

impl StartupProgress {
    pub(super) fn new(
        request: &TerminalRequest,
        observer: ProgressObserver,
        history: watch::Receiver<ProgressHistory>,
    ) -> Self {
        let target = match &request.kind {
            TerminalRequestKind::Attach { target, .. }
            | TerminalRequestKind::Create { target, .. } => target,
        };
        Self {
            // The request has not reached target validation yet. Keep chrome
            // bounded and never put control characters into terminal cells.
            target: target
                .chars()
                .filter(|c| !c.is_control())
                .take(256)
                .collect(),
            observer,
            history,
            cancelling: false,
        }
    }

    async fn changed(&mut self) {
        if self.history.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }

    fn frame(&self, physical_size: TerminalSize) -> ComposedFrame {
        let width = usize::from(
            physical_size
                .columns
                .min(zterm_core::ResourceLimits::default().max_viewport_columns),
        );
        let height = usize::from(
            physical_size
                .rows
                .min(zterm_core::ResourceLimits::default().max_viewport_rows),
        );
        let history = self.history.borrow();
        let stages = history
            .after(0)
            .filter(|(_, event)| !self.cancelling || event.stage != ConnectionStage::Cancelling)
            .map(|(_, event)| event.stage.description().1)
            .collect::<Vec<_>>();
        let mut lines = Vec::new();
        if self.cancelling {
            lines.push(ConnectionStage::Cancelling.description().1.to_owned());
        }
        // On a one-line terminal, keep the latest observation visible.
        if height > lines.len() + 1 {
            lines.push(format!("Target: {}", self.target));
        }
        let remaining = height.saturating_sub(lines.len());
        for stage in stages.iter().skip(stages.len().saturating_sub(remaining)) {
            lines.push((*stage).to_owned());
        }
        let mut rows = BTreeMap::<u16, Vec<TerminalCell>>::new();
        for (row, line) in lines
            .iter()
            .take(usize::from(physical_size.rows))
            .enumerate()
        {
            rows.insert(
                u16::try_from(row).expect("bounded startup rows"),
                text_cells(line, width, TerminalStyle::default()),
            );
        }
        ComposedFrame {
            physical_size,
            layout: LayoutIdentity {
                content_size: TerminalSize::new(
                    u16::try_from(lines.len()).expect("bounded startup rows"),
                    u16::try_from(width).expect("bounded columns"),
                ),
                gutter_column: None,
                status_row: None,
            },
            rows,
            cursor: ComposedCursor {
                row: 0,
                column: 0,
                visible: false,
                style: TerminalStyle::default(),
            },
            modes: TerminalModes::default(),
            colors: Default::default(),
        }
    }

    pub(super) fn present(
        &self,
        output: &mut impl Write,
        presenter: &mut DesktopPresenter,
        size: TerminalSize,
    ) -> Result<(), CliError> {
        presenter.present_startup(output, self.frame(size))?;
        Ok(())
    }
}

/// The two initial waits share scheduling, input and output ownership.
pub(super) enum InactivePresentation<'a> {
    #[cfg(test)]
    None,
    Startup(&'a mut StartupProgress),
    Synchronizing {
        surface: &'a AttachmentSurface,
        viewport: &'a mut ViewportController,
        status: &'a mut StatusRenderer,
    },
}

impl InactivePresentation<'_> {
    pub(super) fn cancelling(&mut self) {
        if let Self::Startup(progress) = self {
            progress.cancelling = true;
            progress.observer.report(ConnectionStage::Cancelling);
        }
    }

    pub(super) async fn changed(&mut self) {
        match self {
            Self::Startup(progress) => progress.changed().await,
            _ => std::future::pending::<()>().await,
        }
    }

    pub(super) fn present(
        &mut self,
        output: &mut impl Write,
        presenter: &mut DesktopPresenter,
        size: TerminalSize,
    ) -> Result<(), CliError> {
        match self {
            #[cfg(test)]
            Self::None => Ok(()),
            Self::Startup(progress) => progress.present(output, presenter, size),
            Self::Synchronizing {
                surface,
                viewport,
                status,
            } => {
                viewport.set_layout(ChromeLayout::new(size, surface.active_screen()));
                status.resize(size);
                status.initial_synchronizing = true;
                if present_surface_with_writer(
                    output,
                    surface,
                    presenter,
                    viewport,
                    status,
                    TerminalViewTransportState::Synchronizing,
                )? {
                    viewport.observe_presentation();
                }
                Ok(())
            }
        }
    }
}
