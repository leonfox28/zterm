package io.github.leonfox28.zterm

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import io.github.leonfox28.zterm.nativebridge.NativeDnsServer
import java.net.Inet6Address

internal class PlatformNetwork(context: Context, private val onChanged: (List<NativeDnsServer>) -> Unit) {
    private val manager = context.getSystemService(ConnectivityManager::class.java)
    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onLinkPropertiesChanged(network: Network, linkProperties: LinkProperties) {
            if (network == manager.activeNetwork) onChanged(servers(linkProperties))
        }
    }
    fun current(): List<NativeDnsServer> = servers(manager.activeNetwork?.let(manager::getLinkProperties))
    fun start() { manager.registerDefaultNetworkCallback(callback) }
    private fun servers(properties: LinkProperties?): List<NativeDnsServer> = properties?.dnsServers.orEmpty()
        .take(8).map { address -> NativeDnsServer(address.address, (address as? Inet6Address)?.scopeId?.toUInt() ?: 0u) }
}
