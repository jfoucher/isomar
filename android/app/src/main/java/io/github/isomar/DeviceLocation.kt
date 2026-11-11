package io.github.isomar

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Looper

/**
 * The phone's own GPS as a source of fixes for the engine (source "device"), through the platform's
 * LocationManager rather than Google Play services, so that the app needs none (F-Droid). Started once the
 * user has granted the location permission.
 */
class DeviceLocation(private val context: Context) : LocationListener {
    private val manager = context.getSystemService(Context.LOCATION_SERVICE) as LocationManager
    private var running = false

    fun granted() = context.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED

    @SuppressLint("MissingPermission")
    fun start() {
        if (running || !granted() || !manager.isProviderEnabled(LocationManager.GPS_PROVIDER)) return
        manager.requestLocationUpdates(LocationManager.GPS_PROVIDER, 1000L, 0f, this, Looper.getMainLooper())
        running = true
    }

    fun stop() {
        if (running) manager.removeUpdates(this)
        running = false
    }

    override fun onLocationChanged(l: Location) {
        Native.location(
            l.latitude,
            l.longitude,
            if (l.hasSpeed()) l.speed * 3600.0 / 1852.0 else Double.NaN,
            if (l.hasBearing()) l.bearing.toDouble() else Double.NaN,
            if (l.hasAccuracy()) l.accuracy.toDouble() else Double.NaN,
            l.time,
        )
    }

    @Deprecated("Called on API levels before 29 only")
    override fun onStatusChanged(provider: String?, status: Int, extras: android.os.Bundle?) {}
}
