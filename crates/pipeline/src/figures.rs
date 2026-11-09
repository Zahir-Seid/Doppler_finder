//! Report figures (PIP-05, FR-OUT-02, v1).
//!
//! Figures are rendered off the compute path: [`render_figures`] writes SVG
//! files into a caller-supplied directory and returns their names for the
//! result payload's `figures` list. Three figures are produced when the data
//! supports them: the whitened spectrogram with fitted fundamental curves,
//! the calibrated 95% uncertainty ellipse, and an evidence panel with the
//! verdict, reason codes, per-event quality and matched aircraft.

use std::path::Path;

use acoustics::StraightPass;
use core_types::Hertz;
use plotters::coord::Shift;
use plotters::prelude::*;

use crate::error::PipelineError;
use crate::run::AnalyzeOutcome;
use crate::stages::Stage;

/// Pixel size of the spectrogram and uncertainty figures.
const FIGURE_W: u32 = 960;
/// See [`FIGURE_W`].
const FIGURE_H: u32 = 540;
/// Heatmap resolution cap: wider maps are block-averaged to this many cells.
const MAX_COLS: usize = 160;
/// See [`MAX_COLS`].
const MAX_ROWS: usize = 90;
/// 95% two-dimensional Gaussian scaling factor, sqrt(chi2_2,0.95).
const ELLIPSE_95: f64 = 2.447_746_8;
/// Plot-area margin inside each figure, in pixels.
const MARGIN: (i64, i64, i64, i64) = (40, 30, 900, 470);

/// Render the report figures for one finished run.
///
/// Writes `spectrogram.svg`, `evidence.svg` and — when a refined solution
/// exists — `uncertainty.svg` into `dir` and returns the file names in
/// drawing order. `sound_speed_m_per_s` is the atmosphere value the fits
/// used, needed to evaluate the fitted curves on the map.
///
/// # Errors
///
/// Returns [`PipelineError::StageFailed`] with [`Stage::Render`] when a file
/// cannot be written or drawn.
pub fn render_figures(
    outcome: &AnalyzeOutcome,
    sound_speed_m_per_s: f64,
    dir: &Path,
) -> Result<Vec<String>, PipelineError> {
    std::fs::create_dir_all(dir).map_err(render_error)?;
    let mut names = Vec::new();

    let spectrogram = dir.join("spectrogram.svg");
    draw_spectrogram(outcome, sound_speed_m_per_s, &spectrogram)?;
    names.push("spectrogram.svg".to_owned());

    if outcome
        .localize
        .as_ref()
        .is_some_and(|l| l.refined.is_some() && l.ellipse_95.is_some())
    {
        let uncertainty = dir.join("uncertainty.svg");
        draw_uncertainty(outcome, &uncertainty)?;
        names.push("uncertainty.svg".to_owned());
    }

    let evidence = dir.join("evidence.svg");
    draw_evidence(outcome, &evidence)?;
    names.push("evidence.svg".to_owned());

    Ok(names)
}

fn render_error(error: impl std::fmt::Display) -> PipelineError {
    PipelineError::StageFailed {
        stage: Stage::Render,
        message: error.to_string(),
    }
}

/// Convert a bounded pixel coordinate to `i32`, saturating at the edges.
fn px(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// Convert a frame/bin count to `i64` without a wrapping cast.
fn idx(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Open an SVG drawing area, filling it white.
fn canvas(
    path: &Path,
    size: (u32, u32),
) -> Result<DrawingArea<SVGBackend<'_>, Shift>, PipelineError> {
    let backend = SVGBackend::new(path, size).into_drawing_area();
    backend
        .fill(&WHITE)
        .map_err(|e| render_error(format!("fill: {e}")))?;
    Ok(backend)
}

/// Whitened z-map heatmap with each retained event's fitted fundamental
/// curve overlaid. z is clamped to `[-2, 12]` for display; the title carries
/// the verdict and top-cluster mass.
fn draw_spectrogram(
    outcome: &AnalyzeOutcome,
    sound_speed_m_per_s: f64,
    path: &Path,
) -> Result<(), PipelineError> {
    let map = &outcome.zmap;
    let frames = map.frame_count();
    let bins = map.bin_count();
    if frames == 0 || bins == 0 {
        return Err(render_error("empty spectral map"));
    }
    let backend = canvas(path, (FIGURE_W, FIGURE_H))?;
    let (plot_x, plot_y, plot_w, plot_h) = MARGIN;

    draw_heatmap(&backend, map, frames, bins, plot_x, plot_y, plot_w, plot_h)?;
    draw_curves(
        &backend,
        outcome,
        map,
        frames,
        bins,
        sound_speed_m_per_s,
        plot_x,
        plot_y,
        plot_w,
        plot_h,
    )?;

    // Title: verdict and top-cluster mass.
    let (verdict, mass) = outcome.localize.as_ref().map_or_else(
        || ("no localization".to_owned(), 0.0),
        |l| {
            (
                format!("{:?}", l.decision.verdict),
                l.clusters
                    .first()
                    .map_or(0.0, |cluster| cluster.posterior_mass),
            )
        },
    );
    backend
        .draw(&Text::new(
            format!("{verdict}  top mass {mass:.3}"),
            (px(plot_x), 12),
            ("sans-serif", 18).into_font(),
        ))
        .map_err(|e| render_error(format!("title: {e}")))?;
    backend
        .present()
        .map_err(|e| render_error(format!("present: {e}")))?;
    Ok(())
}

/// Block-averaged z-map heatmap, dark where z is strongly positive.
#[allow(clippy::too_many_arguments)]
fn draw_heatmap(
    backend: &DrawingArea<SVGBackend<'_>, Shift>,
    map: &dsp::ZMap,
    frames: usize,
    bins: usize,
    plot_x: i64,
    plot_y: i64,
    plot_w: i64,
    plot_h: i64,
) -> Result<(), PipelineError> {
    let sx = (bins / MAX_COLS).max(1);
    let sy = (frames / MAX_ROWS).max(1);
    let out_cols = bins.div_ceil(sx);
    let out_rows = frames.div_ceil(sy);

    for row in 0..out_rows {
        for col in 0..out_cols {
            let mut sum = 0.0_f64;
            let mut count = 0_u32;
            for f in (row * sy)..((row * sy + sy).min(frames)) {
                for b in (col * sx)..((col * sx + sx).min(bins)) {
                    if let Some(value) = map.value(f, b) {
                        sum += f64::from(value);
                        count += 1;
                    }
                }
            }
            if count == 0 {
                continue;
            }
            let z = (sum / f64::from(count)).clamp(-2.0, 12.0);
            // Dark = strong positive z (a curve), light = noise floor.
            let shade = ((z + 2.0) / 14.0).clamp(0.0, 1.0);
            let gray = (255.0 * (1.0 - shade)) as u8;
            let f0 = row * sy;
            let f1 = (row * sy + sy).min(frames);
            let b0 = col * sx;
            let b1 = (col * sx + sx).min(bins);
            let x0 = plot_x + idx(b0) * plot_w / idx(bins);
            let x1 = plot_x + idx(b1) * plot_w / idx(bins);
            let y0 = plot_y + idx(f0) * plot_h / idx(frames);
            let y1 = plot_y + idx(f1) * plot_h / idx(frames);
            backend
                .draw(&Rectangle::new(
                    [(px(x0), px(y0)), (px(x1), px(y1))],
                    RGBColor(gray, gray, gray).filled(),
                ))
                .map_err(|e| render_error(format!("rect: {e}")))?;
        }
    }
    Ok(())
}

/// One polyline per retained event: the fitted fundamental evaluated on the
/// emission-time model at every frame.
#[allow(clippy::too_many_arguments)]
fn draw_curves(
    backend: &DrawingArea<SVGBackend<'_>, Shift>,
    outcome: &AnalyzeOutcome,
    map: &dsp::ZMap,
    frames: usize,
    bins: usize,
    sound_speed_m_per_s: f64,
    plot_x: i64,
    plot_y: i64,
    plot_w: i64,
    plot_h: i64,
) -> Result<(), PipelineError> {
    let max_freq = map.bin_hz(bins.saturating_sub(1)).unwrap_or(1.0).max(1.0);
    for event in outcome
        .events
        .iter()
        .filter(|event| event.assessment.retained)
    {
        let fit = &event.fit;
        let pass = StraightPass {
            closest_approach_m: fit.closest_m.0,
            speed_m_per_s: fit.speed_m_per_s,
        };
        let mut points: Vec<(i32, i32)> = Vec::with_capacity(frames);
        for (frame, &time_s) in map.frame_times_s().iter().enumerate() {
            let t_rel = time_s - fit.heard_at.0;
            let Ok(received) = pass.received_offset_from_heard_offset(t_rel, sound_speed_m_per_s)
            else {
                continue;
            };
            let Ok(freq) = pass.received_frequency(
                Hertz(fit.fundamental_hz.0),
                1,
                received.0,
                sound_speed_m_per_s,
            ) else {
                continue;
            };
            let freq = freq.0;
            if !freq.is_finite() || freq <= 0.0 {
                continue;
            }
            let x = plot_x + idx(frame) * plot_w / idx(frames.max(1));
            let y = plot_y + plot_h
                - (freq / max_freq * plot_h as f64)
                    .clamp(0.0, plot_h as f64)
                    .round() as i64;
            points.push((px(x), px(y)));
        }
        if points.len() >= 2 {
            backend
                .draw(&PathElement::new(points, RED))
                .map_err(|e| render_error(format!("curve: {e}")))?;
        }
    }
    Ok(())
}

/// The calibrated 95% ellipse around the refined observer, drawn in its own
/// East-North frame with the semi-major axis labelled.
fn draw_uncertainty(outcome: &AnalyzeOutcome, path: &Path) -> Result<(), PipelineError> {
    let Some(localize) = &outcome.localize else {
        return Ok(());
    };
    let (Some(_refined), Some(ellipse)) = (&localize.refined, localize.ellipse_95) else {
        return Ok(());
    };
    let (a, b, c) = (ellipse[0][0], ellipse[0][1], ellipse[1][1]);
    if !(a.is_finite() && b.is_finite() && c.is_finite()) {
        return Err(render_error("non-finite ellipse"));
    }
    let tr = a + c;
    let det = a * c - b * b;
    let disc = (tr * tr - 4.0 * det).max(0.0).sqrt();
    let lambda_max = f64::midpoint(tr, disc).max(0.0);
    let lambda_min = f64::midpoint(tr, -disc).max(0.0);
    let semi_major = ELLIPSE_95 * lambda_max.sqrt();

    let backend = canvas(path, (FIGURE_W, FIGURE_H))?;
    // Scale the ellipse to fit with margin.
    let scale = (200.0 / semi_major.max(1.0)).min(1.0);
    let cx = px(i64::from(FIGURE_W) / 2);
    let cy = px(i64::from(FIGURE_H) / 2);
    let s1 = ELLIPSE_95 * lambda_max.sqrt() * scale;
    let s2 = ELLIPSE_95 * lambda_min.sqrt() * scale;
    let angle = 0.5 * (2.0 * b).atan2(a - c);
    let (sin_a, cos_a) = angle.sin_cos();

    let mut points: Vec<(i32, i32)> = Vec::with_capacity(65);
    for step in 0..=64_i64 {
        let theta = step as f64 / 64.0 * std::f64::consts::TAU;
        let (sin_t, cos_t) = theta.sin_cos();
        let ex = cos_t * s1;
        let ny = sin_t * s2;
        let east = cos_a * ex - sin_a * ny;
        let north = sin_a * ex + cos_a * ny;
        points.push((
            px(i64::from(cx) + east.round() as i64),
            px(i64::from(cy) - north.round() as i64),
        ));
    }
    backend
        .draw(&PathElement::new(points, BLUE))
        .map_err(|e| render_error(format!("ellipse: {e}")))?;
    backend
        .draw(&Circle::new((cx, cy), 4, RED.filled()))
        .map_err(|e| render_error(format!("centre: {e}")))?;
    backend
        .draw(&Text::new(
            format!("95% semi-major {semi_major:.0} m"),
            (px(i64::from(cx) - 120), 20),
            ("sans-serif", 16).into_font(),
        ))
        .map_err(|e| render_error(format!("label: {e}")))?;
    backend
        .present()
        .map_err(|e| render_error(format!("present: {e}")))?;
    Ok(())
}

/// Text panel: verdict, reason codes, per-event quality, matched aircraft.
fn draw_evidence(outcome: &AnalyzeOutcome, path: &Path) -> Result<(), PipelineError> {
    let backend = canvas(path, (720, 480))?;
    let mut y = 30_i64;
    let mut line = |text: String| -> Result<(), PipelineError> {
        backend
            .draw(&Text::new(
                text,
                (20, px(y)),
                ("sans-serif", 14).into_font(),
            ))
            .map_err(|e| render_error(format!("text: {e}")))?;
        y += 22;
        Ok(())
    };

    let (verdict, reasons) = outcome.localize.as_ref().map_or_else(
        || {
            (
                "NotLocatable".to_owned(),
                vec!["no_flight_match".to_owned()],
            )
        },
        |l| {
            (
                format!("{:?}", l.decision.verdict),
                l.decision
                    .reason_codes
                    .iter()
                    .map(|code| format!("{code:?}").to_lowercase())
                    .collect(),
            )
        },
    );
    line(format!("verdict: {verdict}"))?;
    line(format!("reasons: {}", reasons.join(", ")))?;
    line(format!(
        "detections: {}  retained events: {}",
        outcome.detections.len(),
        outcome
            .events
            .iter()
            .filter(|event| event.assessment.retained)
            .count()
    ))?;
    for (index, event) in outcome.events.iter().enumerate() {
        line(format!(
            "event {index}: quality {:.2} retained {} snr {:.1} dB — {}",
            event.assessment.quality.score,
            event.assessment.retained,
            event.assessment.curve_snr_db,
            event.assessment.explanation()
        ))?;
    }
    backend
        .present()
        .map_err(|e| render_error(format!("present: {e}")))?;
    Ok(())
}
