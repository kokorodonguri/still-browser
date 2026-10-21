package app.still.browser

import java.net.IDN

/** Address-bar rules shared with the desktop build (`core/src/lib.rs`). */
object Address {
    const val BLANK = "about:blank"
    /** Longest address accepted for navigation or as the home page. */
    const val URL_LIMIT = 32_000

    /** http(s) URLs pass through, bare hosts get https://, everything else is a search. */
    fun normalize(input: String, engine: SearchEngine = SearchEngine.Google): String {
        val text = input.trim()
        if (text.isEmpty()) return BLANK
        return webUrl(text) ?: engine.searchUrl(encodeComponent(text))
    }

    /** The web address [input] names (an http(s) URL or a bare host), if any. */
    fun webUrl(input: String): String? {
        val text = input.trim()
        parseHttp(text)?.let { return it }
        if (text.none(Char::isWhitespace) && '.' in text) {
            val url = parseHttp("https://$text") ?: return null
            if ('.' in hostOf(url)) return url
        }
        return null
    }

    /** Short label for a tab whose page has not reported a title yet. */
    fun hostLabel(url: String): String = parseHttp(url)?.let(::hostOf)?.ifEmpty { null } ?: ""

    fun isWeb(url: String): Boolean =
        url.startsWith("https://", ignoreCase = true) || url.startsWith("http://", ignoreCase = true)

    /** Percent-encodes everything except ASCII letters and digits, like the desktop build. */
    fun encodeComponent(text: String): String {
        val out = StringBuilder()
        for (b in text.toByteArray(Charsets.UTF_8)) {
            val c = b.toInt() and 0xff
            if (c in 'a'.code..'z'.code || c in 'A'.code..'Z'.code || c in '0'.code..'9'.code) {
                out.append(c.toChar())
            } else {
                out.append('%').append(HEX[c shr 4]).append(HEX[c and 15])
            }
        }
        return out.toString()
    }

    private const val HEX = "0123456789ABCDEF"
    private val SCHEME = Regex("^([A-Za-z][A-Za-z0-9+.-]*):(.*)$", RegexOption.DOT_MATCHES_ALL)
    private val HOST_LABEL = Regex("^[a-z0-9-]+$")
    private val IPV6 = Regex("^\\[[0-9a-f:.]+]$")

    private fun hostOf(url: String): String {
        val rest = url.substringAfter("://")
        val authority = rest.substring(0, rest.indexOfAny(charArrayOf('/', '?', '#')).let { if (it < 0) rest.length else it })
        val host = authority.substringAfterLast('@')
        return if (host.startsWith('[')) host.substringBefore(']') + "]" else host.substringBefore(':')
    }

    /**
     * A canonical http(s) URL, or null. Hosts are lower-cased and IDN-encoded,
     * default ports dropped, an empty path becomes "/", and spaces or non-ASCII
     * characters in the path are percent-encoded.
     */
    private fun parseHttp(text: String): String? {
        if (text.length > URL_LIMIT) return null
        val cleaned = text.trim().filterNot { it == '\t' || it == '\n' || it == '\r' }
        val match = SCHEME.matchEntire(cleaned) ?: return null
        val scheme = match.groupValues[1].lowercase()
        if (scheme != "http" && scheme != "https") return null
        val afterScheme = match.groupValues[2].trimStart('/', '\\')
        val end = afterScheme.indexOfAny(charArrayOf('/', '\\', '?', '#')).let { if (it < 0) afterScheme.length else it }
        val authority = afterScheme.substring(0, end)
        var tail = afterScheme.substring(end)
        val at = authority.lastIndexOf('@')
        val userInfo = if (at >= 0) authority.substring(0, at + 1) else ""
        val hostPort = authority.substring(at + 1)
        val (rawHost, port) = splitPort(hostPort) ?: return null
        val host = canonicalHost(rawHost) ?: return null
        if (userInfo.any { it.isWhitespace() }) return null
        val portPart = when (port) {
            null -> ""
            if (scheme == "https") 443 else 80 -> ""
            else -> ":$port"
        }
        if (tail.startsWith('\\')) tail = "/" + tail.substring(1)
        if (tail.isEmpty() || tail[0] == '?' || tail[0] == '#') tail = "/$tail"
        return "$scheme://$userInfo$host$portPart${encodeLoose(tail)}"
    }

    private fun splitPort(hostPort: String): Pair<String, Int?>? {
        val colon = if (hostPort.startsWith('[')) {
            val close = hostPort.indexOf(']')
            if (close < 0) return null
            if (close + 1 < hostPort.length && hostPort[close + 1] != ':') return null
            if (close + 1 < hostPort.length) close + 1 else -1
        } else {
            hostPort.lastIndexOf(':')
        }
        if (colon < 0) return hostPort to null
        val digits = hostPort.substring(colon + 1)
        if (digits.isEmpty()) return hostPort.substring(0, colon) to null
        if (digits.length > 5 || digits.any { it !in '0'..'9' }) return null
        val port = digits.toInt()
        if (port > 65535) return null
        return hostPort.substring(0, colon) to port
    }

    private fun canonicalHost(raw: String): String? {
        if (raw.isEmpty()) return null
        val lower = raw.lowercase()
        if (lower.startsWith('[')) return lower.takeIf { IPV6.matches(it) }
        val ascii = try {
            IDN.toASCII(lower, IDN.ALLOW_UNASSIGNED).lowercase()
        } catch (_: IllegalArgumentException) {
            return null
        }
        val labels = ascii.removeSuffix(".").split('.')
        if (labels.any { it.isEmpty() || !HOST_LABEL.matches(it) }) return null
        return ascii
    }

    /** Encodes characters a URL may not contain, leaving existing escapes alone. */
    private fun encodeLoose(part: String): String {
        val out = StringBuilder(part.length)
        var i = 0
        while (i < part.length) {
            val cp = part.codePointAt(i)
            i += Character.charCount(cp)
            if (cp in 0x21..0x7e && cp != '"'.code && cp != '<'.code && cp != '>'.code && cp != '`'.code) {
                out.append(cp.toChar())
            } else {
                for (b in String(Character.toChars(cp)).toByteArray(Charsets.UTF_8)) {
                    val c = b.toInt() and 0xff
                    out.append('%').append(HEX[c shr 4]).append(HEX[c and 15])
                }
            }
        }
        return out.toString()
    }
}
