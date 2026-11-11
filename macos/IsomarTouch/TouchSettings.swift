import SwiftUI

/// The boat's instruments on a phone or tablet: a list to tap, the Mac's editor in a sheet. Serial ports are
/// not offered (an iPhone has none); TCP over the boat's Wi-Fi asks for the local network once.
struct TouchConnections: View {
    @ObservedObject var model = IsomarModel.shared
    @State private var rows: [Connection] = []
    @State private var editing: Connection?
    @State private var monitor = false
    @State private var timer: Timer?

    var body: some View {
        List {
            Section {
                ForEach(rows) { c in
                    Button { editing = c } label: {
                        HStack(spacing: 12) {
                            Circle().fill(color(c)).frame(width: 10, height: 10)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(c.name).foregroundStyle(Color.isomar(c.enabled ? .text : .textDisabled))
                                Text([c.describe, c.status].filter { !$0.isEmpty }.joined(separator: " · ")).font(.system(size: 13)).foregroundStyle(Color.isomar(.textMuted))
                            }
                        }
                    }
                    .swipeActions { Button(role: .destructive) { model.send("connection_remove", ["id": c.id]); load() } label: { Label("Remove", systemImage: "trash") } }
                }
                if rows.isEmpty { Text("No connections yet. Without any, the phone's own GPS gives the position.").foregroundStyle(Color.isomar(.textMuted)) }
            } footer: {
                Text("Instruments and GPS send NMEA 0183 over the boat's Wi-Fi (a TCP server, gpsd or Signal K), or from a file.")
            }
            Section {
                Button { editing = Connection() } label: { Label("Add a connection", systemImage: "plus") }
                Button { monitor = true } label: { Label("Data monitor", systemImage: "waveform.path.ecg") }
            }
        }
        .navigationTitle("Connections")
        .sheet(item: $editing) { c in
            ConnectionEditor(connection: c) { saved in
                if let saved, let e = model.send("connection_save", saved.json) { IsomarModel.alert("Could not save the connection", e) }
                editing = nil
                load()
            }
        }
        .sheet(isPresented: $monitor) { NavigationStack { DataMonitorView().navigationTitle("Data monitor").toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { monitor = false } } } } }
        .onAppear { load(); timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { _ in load() } }
        .onDisappear { timer?.invalidate() }
    }

    private func color(_ c: Connection) -> Color { Color.isomar(c.status.hasPrefix("Connected") ? .ok : (c.status.hasPrefix("Disc") ? .alarm : .textDisabled)) }
    private func load() { rows = model.list("connections").compactMap { Connection(row: $0) } }
}

/// What the app is, where its source is (COPYING.appstore asks the app to say), and whose data it draws.
struct TouchAbout: View {
    static let source = URL(string: "https://isomar.marinetechs.com/")!
    @ObservedObject var model = IsomarModel.shared

    var body: some View {
        List {
            Section {
                Text("A chart plotter for sailors. Free software under the GNU General Public License, version 3 or later, with an additional permission for app stores.")
                Text("Not for navigation unless an official chart you have licensed covers the area.").font(.system(size: 15, weight: .semibold))
            }
            Section("Data") {
                Text("OpenStreetMap contributors (ODbL), Open Waters seamap tiles (CC BY 4.0), OpenFreeMap and OpenMapTiles, EMODnet Bathymetry, NOAA GFS forecasts, ECMWF open data forecasts (CC BY 4.0), SHOM open data (Licence Ouverte).").font(.system(size: 13))
            }
            Section {
                Button("Website and source code") { Platform.open(Self.source) }
            }
        }
        .navigationTitle("Isomar \(Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "")")
    }
}
