import SwiftUI

/// Sailing or motoring: the user's switch, or the engine's word while one reports. What the performance log
/// learns from, so the time under engine does not count as sailing.
struct SailSwitch: View {
    let sail: PerfSwitch
    @ObservedObject var model: IsomarModel
    var compact = false

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Picker("", selection: Binding(get: { sail.mode }, set: { set($0) })) {
                Text(compact ? "Sail" : "Sailing").tag("sailing")
                Text(compact ? "Engine" : "Motoring").tag("motoring")
            }
            .pickerStyle(.segmented).labelsHidden()
            .disabled(sail.byEngine)
            if let n = sail.note ?? sail.hint, !compact {
                Text(n).font(.caption).foregroundStyle(Color.isomar(.textMuted))
            }
        }
        .help(sail.note ?? sail.hint ?? "")
    }

    private func set(_ mode: String) {
        if let e = model.send("perf_set_mode", ["mode": mode]) { model.toast = e }
    }
}

/// The Performance page, pushed from Instruments: what the log holds, what it says about the instruments,
/// the polar learnt from them, how fast the boat sails against its polar, how good the forecast was, waves,
/// and passages planned against sailed. Built from `vm::performance` through `perf_get` and `perf_page`.
struct PerformancePage: View {
    @ObservedObject var model: IsomarModel
    @State private var column = 2
    @State private var confirmClear = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            BackBar(title: "Performance", model: model)
            ScrollView {
                if let p = model.performance { content(p) } else { ProgressView().padding() }
            }
        }
        .onAppear {
            // Asking for the record starts an analysis when the log is newer than the last one.
            model.performance = model.call("perf_get", as: PerformanceInfo?.self) ?? nil
        }
        .confirmationDialog("Clear the performance log?", isPresented: $confirmClear) {
            Button("Clear", role: .destructive) { model.send("perf_clear") }
        } message: { Text("Every sample logged against the polar in use, and every passage, is deleted. The polars and the calibration are kept.") }
    }

    @ViewBuilder private func content(_ p: PerformanceInfo) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(p.summary).font(.system(size: 12)).foregroundStyle(Color.isomar(.textMuted))
                if p.analysing { ProgressView().controlSize(.small) }
                Spacer()
                Button { model.job("perf_analyse") { model.send("perf_analyse") } } label: { if model.busy("perf_analyse") { ProgressView().controlSize(.small) } else { Image(systemName: "arrow.clockwise") } }
                    .buttonStyle(.platformLink).modifier(JobBusy(busy: model.busy("perf_analyse"))).help("Analyse again")
            }.padding(.horizontal, 14)
            // The log is the polar in use's: it is measured against it and learns into it alone.
            HStack {
                Text("Logged against the polar \(p.polar)").font(.caption)
                Spacer()
                Button("Change…") { PolarWindow.open() }.buttonStyle(.platformLink).font(.caption)
            }.padding(.horizontal, 14)
            if let e = p.error { Text("The log could not be read: \(e)").font(.caption).padding(.horizontal, 14) }

            SectionHeader(title: "Log") { EmptyView() }
            Toggle("Record while under way", isOn: Binding(get: { p.recording }, set: { model.send("perf_set_recording", ["on": $0]) })).padding(.horizontal, 14)
            lines(p.hours, trailing: true)
            HStack {
                Button("Export…") { export() }
                if !p.empty { Button("Clear…", role: .destructive) { confirmClear = true } }
            }.controlSize(.small).padding(.horizontal, 14)

            if let c = p.calibration {
                SectionHeader(title: "Instruments") { EmptyView() }
                Text(c.evidence).font(.caption).foregroundStyle(Color.isomar(.textMuted)).padding(.horizontal, 14)
                lines(c.findings)
                HStack {
                    if c.canSave { Button("Correct the analysis with this") { if let e = model.send("perf_calibration_save") { IsomarModel.alert("Could not save the calibration", e) } } }
                    if c.saved { Button("Forget the saved correction") { model.send("perf_calibration_clear") } }
                }.controlSize(.small).padding(.horizontal, 14)
                Text("The correction applies to this analysis only; it does not change what the instruments show.").font(.caption2).foregroundStyle(Color.isomar(.textMuted)).padding(.horizontal, 14)
            }

            if let l = p.learned { learned(l, canUndo: p.canUndoPolar) }

            if p.suggestion != nil || !p.factors.isEmpty {
                SectionHeader(title: "Performance") { EmptyView() }
                if let s = p.suggestion {
                    Text(s.text).font(.system(size: 12)).padding(.horizontal, 14)
                    if s.applied {
                        Text("Weather routing starts from this.").font(.caption).foregroundStyle(Color.isomar(.textMuted)).padding(.horizontal, 14)
                    } else {
                        Button(s.button) { model.send("perf_apply_factor", ["pct": s.pct]); model.toast = "Weather routing will start from this performance" }
                            .buttonStyle(.platformProminent).controlSize(.small).padding(.horizontal, 14)
                    }
                }
                ForEach(p.factors, id: \.self) { g in
                    DisclosureGroup {
                        ForEach(g.rows, id: \.self) { r in
                            VStack(alignment: .leading, spacing: 1) {
                                Text(r.label).font(.system(size: 12))
                                Text(r.text).font(.system(size: 11)).foregroundStyle(Color.isomar(.textMuted))
                            }.frame(maxWidth: .infinity, alignment: .leading)
                        }
                    } label: {
                        VStack(alignment: .leading, spacing: 1) {
                            Text(g.title).font(.system(size: 12, weight: .semibold))
                            Text(g.note).font(.system(size: 11)).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
                        }
                    }.padding(.horizontal, 14).padding(.vertical, 2)
                }
            }

            if !p.forecastCheck.isEmpty || p.hindcastPending > 0 {
                SectionHeader(title: "Forecast against the instruments") { EmptyView() }
                lines(p.forecastCheck)
                if p.hindcastPending > 0 {
                    Button { model.job("perf_backfill") { model.send("perf_backfill") } } label: { JobLabel(title: "Fetch the wind as analysed afterwards", busy: model.busy("perf_backfill")) }
                        .controlSize(.small).modifier(JobBusy(busy: model.busy("perf_backfill"))).padding(.horizontal, 14)
                        .help("For \(p.hindcastPending) samples of the last nine days: the model's analysis, closer to the wind that blew than the forecast was")
                }
            }
            if !p.waves.isEmpty {
                SectionHeader(title: "Waves") { EmptyView() }
                lines(p.waves)
            }
            if !p.passages.isEmpty {
                SectionHeader(title: "Passages") { EmptyView() }
                lines(p.passages)
            }
        }
        .padding(.bottom, 12)
    }

    @ViewBuilder private func lines(_ ls: [PerfLine], trailing: Bool = false) -> some View {
        ForEach(ls, id: \.self) { l in
            if trailing {
                HStack { Text(l.title).font(.system(size: 12)); Spacer(); Text(l.detail).font(.numeric(12, weight: .regular)).foregroundStyle(Color.isomar(.textMuted)) }
                    .padding(.horizontal, 14).padding(.vertical, 1)
            } else {
                VStack(alignment: .leading, spacing: 1) {
                    Text(l.title).font(.system(size: 12))
                    Text(l.detail).font(.system(size: 11)).foregroundStyle(Color.isomar(.textMuted)).fixedSize(horizontal: false, vertical: true)
                }.padding(.horizontal, 14).padding(.vertical, 2)
            }
        }
    }

    @ViewBuilder private func learned(_ l: PerfLearned, canUndo: Bool) -> some View {
        SectionHeader(title: "Polar") { EmptyView() }
        Text(l.summary).font(.caption).foregroundStyle(Color.isomar(.textMuted)).padding(.horizontal, 14)
        Picker("", selection: $column) {
            ForEach(Array(l.columns.enumerated()), id: \.offset) { i, w in Text("\(Int(w))").tag(i) }
        }
        .pickerStyle(.segmented).labelsHidden().padding(.horizontal, 14)
        if column < l.curves.count {
            CurveView(curve: l.curves[column]).frame(height: 220).padding(.horizontal, 14)
            if let b = l.curves[column].best { Text(b).font(.caption).foregroundStyle(Color.isomar(.textMuted)).padding(.horizontal, 14) }
        }
        Text("Grey: the boat's polar now. Blue: as learnt. Dots: what the boat did (the top of each 5° bin), the bolder the more there is.")
            .font(.caption2).foregroundStyle(Color.isomar(.textMuted)).padding(.horizontal, 14)
        Text("Coverage: where the log has data").font(.system(size: 11, weight: .semibold)).padding(.horizontal, 14).padding(.top, 6)
        CoverageView(coverage: l.coverage, columns: l.columns).frame(height: CGFloat(16 + 12 * l.columns.count)).padding(.horizontal, 14)
        if let c = l.caution { Text(c).font(.caption).foregroundStyle(Color.isomar(.warning)).padding(.horizontal, 14).padding(.top, 4) }
        HStack {
            if l.changes > 0 {
                Button("Use the learnt values (\(l.changes) cells)") {
                    if let d = model.dict("perf_apply_polar") {
                        if let e = d["error"] as? String { IsomarModel.alert("Could not change the polar", e) } else { model.toast = "\(d["changed"] as? Int ?? 0) speeds of the polar changed" }
                    }
                }
                .buttonStyle(.platformProminent).tint(l.caution == nil ? .accentColor : .gray)
            }
            Button("Open the polar editor") { PolarWindow.open() }
            if canUndo { Button("Back to the polar before") { if let e = model.send("perf_undo_polar") { IsomarModel.alert("Could not go back", e) } } }
        }
        .controlSize(.small).padding(.horizontal, 14).padding(.top, 4)
    }

    private func export() {
        model.choose(open: false, types: ["csv"], name: "performance-log.csv") { path in
            if let d = model.dict("perf_export_csv", ["path": path]) {
                if let e = d["error"] as? String { IsomarModel.alert("Could not write the file", e) } else { model.toast = "\(d["rows"] as? Int ?? 0) samples exported" }
            }
        }
    }
}

/// Half a polar diagram, starboard side: the wind from the top, speed outwards.
struct CurveView: View {
    let curve: PerfCurve

    var body: some View {
        Canvas { ctx, size in
            let speeds = (curve.current + curve.learned + curve.cells).map { $0.count > 1 ? $0[1] : 0 }
            let top = max(1, speeds.max() ?? 1)
            let step = top < 11 ? 2.0 : 5.0
            let scale = (top / step).rounded(.up) * step
            let r = min(size.height / 2 - 8, size.width * 0.75)
            let c = CGPoint(x: max(4, (size.width - r) / 2), y: size.height / 2)
            func at(_ a: Double, _ kn: Double) -> CGPoint {
                let rr = kn / scale * r
                return CGPoint(x: c.x + CGFloat(sin(a * .pi / 180) * rr), y: c.y - CGFloat(cos(a * .pi / 180) * rr))
            }
            for k in stride(from: step, through: scale, by: step) {
                var ring = Path()
                ring.addArc(center: c, radius: k / scale * r, startAngle: .degrees(-90), endAngle: .degrees(90), clockwise: false)
                ctx.stroke(ring, with: .color(.gray.opacity(0.25)), lineWidth: 0.5)
            }
            func path(_ pts: [[Double]]) -> Path {
                var p = Path()
                for (i, q) in pts.enumerated() where q.count > 1 {
                    i == 0 ? p.move(to: at(q[0], q[1])) : p.addLine(to: at(q[0], q[1]))
                }
                return p
            }
            ctx.stroke(path(curve.current), with: .color(.gray), lineWidth: 1.5)
            for q in curve.cells where q.count > 2 {
                let pt = at(q[0], q[1])
                ctx.fill(Path(ellipseIn: CGRect(x: pt.x - 3, y: pt.y - 3, width: 6, height: 6)), with: .color(Color.accentColor.opacity(0.15 + 0.6 * q[2])))
            }
            ctx.stroke(path(curve.learned), with: .color(.accentColor), lineWidth: 2.5)
        }
    }
}

/// Wind speeds (rows) by 5 degree bins of angle (columns), tinted by how much the log has there.
struct CoverageView: View {
    let coverage: [[Double]]
    let columns: [Double]

    var body: some View {
        Canvas { ctx, size in
            let left: CGFloat = 36
            let rows = CGFloat(max(1, coverage.count))
            let bins = CGFloat(coverage.first?.count ?? 36)
            let cw = (size.width - left) / bins
            let ch = (size.height - 14) / rows
            for (j, row) in coverage.enumerated() {
                let y = CGFloat(j) * ch
                ctx.draw(Text("\(Int(j < columns.count ? columns[j] : 0)) kn").font(.system(size: 9)), at: CGPoint(x: 0, y: y + ch / 2), anchor: .leading)
                for (i, v) in row.enumerated() {
                    let rect = CGRect(x: left + CGFloat(i) * cw, y: y + 1, width: cw - 1, height: ch - 2)
                    ctx.fill(Path(rect), with: .color(v > 0 ? Color.accentColor.opacity(0.15 + 0.85 * v) : Color.gray.opacity(0.1)))
                }
            }
            for deg in [0.0, 90.0, 180.0] {
                let x = left + CGFloat(deg / 180) * (size.width - left)
                ctx.draw(Text("\(Int(deg))°").font(.system(size: 9)), at: CGPoint(x: min(x, size.width - 12), y: size.height - 6))
            }
        }
    }
}
