#include "session.h"

#include <cmath>

// The pinned SDK tests only macro existence before emitting a Clang-only
// attribute. Native MSVC defines __has_attribute without that syntax.
#if defined(_MSC_VER) && !defined(__clang__)
#pragma push_macro("__has_attribute")
#undef __has_attribute
#endif
#include <WebView2EnvironmentOptions.h>
#if defined(_MSC_VER) && !defined(__clang__)
#pragma pop_macro("__has_attribute")
#endif

#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QHostAddress>
#include <QJsonArray>
#include <QJsonDocument>
#include <QRegularExpression>
#include <QSaveFile>
#include <QSet>

using Microsoft::WRL::Callback;
using Microsoft::WRL::ComPtr;
using Microsoft::WRL::Make;

namespace
{
    constexpr qsizetype maxResponseBytes = 256 * 1024 - 1; // reserve the JSON-lines newline
    constexpr wchar_t windowClass[] = L"qbuttWebSession";
    const QUrl proxyCheckUrl {QStringLiteral("http://qbutt-webview.invalid/")};
    constexpr wchar_t documentState[] = LR"JS((() => {
        const has = selector => document.querySelector(selector) !== null;
        const anubis = has('script#anubis_challenge[type="application/json"]')
            && has('script#anubis-main[src*="/.within.website/x/cmd/anubis/"]');
        const cloudflare = has('#challenge-form, #cf-challenge-running, #challenge-stage, .cf-turnstile');
        const ddg = has('script[src*="check.ddos-guard.net/check.js"]');
        const captcha = has(`form#captcha-form, form[action*="/captcha"], input[name="cap_sid"],
            input[name="g-recaptcha-response"], textarea[name="g-recaptcha-response"],
            input[name="h-captcha-response"], .g-recaptcha, .h-captcha`);
        const login = location.pathname === '/forum/login.php'
            || (has('input[name="login_username"]') && has('input[name="login_password"]')
                && !has('#logged-in-username'));
        return {url: location.href, ready: document.readyState === 'complete',
            challenge: anubis || cloudflare || ddg || captcha || login};
    })())JS";

    QString consumeString(LPWSTR value)
    {
        const QString result = QString::fromWCharArray(value);
        CoTaskMemFree(value);
        return result;
    }
}

WebSession::WebSession(std::function<void(const QJsonObject &)> completed, QObject *parent)
    : QObject {parent}
    , m_completed {std::move(completed)}
{
    m_deadline.setSingleShot(true);
    m_poll.setInterval(250);
    connect(&m_deadline, &QTimer::timeout, this, [this]
    {
        fail(m_sawChallenge ? QStringLiteral("browser_check_required") : QStringLiteral("webview_timeout"));
    });
    connect(&m_poll, &QTimer::timeout, this, [this] { pollDocument(); });
}

WebSession::~WebSession()
{
    close();
}

QString WebSession::id() const
{
    return m_id;
}

void WebSession::start(const QJsonObject &request)
{
    if (m_finished)
        return;
    if (m_started)
    {
        fail(QStringLiteral("webview_failed"));
        return;
    }
    m_started = true;
    m_id = request.value("id").toString();
    m_url = QUrl {request.value("url").toString(), QUrl::StrictMode};
    const QString mode = request.value("mode").toString();
    m_interactive = (mode == "interactive");
    const QString profile = QDir::cleanPath(request.value("profileDirectory").toString());
    const QJsonObject proxy = request.value("proxy").toObject();
    m_proxy = QUrl {proxy.value("url").toString(), QUrl::StrictMode};
    m_proxyUsername = proxy.value("username").toString();
    m_proxyPassword = proxy.value("password").toString();
    const int timeout = request.value("timeoutMs").toInt();
    const QFileInfo profileInfo {profile};
    const QRegularExpression profileName {"\\A[a-f0-9]{64}\\z"};
    const QString fingerprint = request.value("configuredSession").toString();
    const QJsonArray initialCookies = request.value("initialCookies").toArray();
    const bool authoritative = request.value("authoritativeCookies").toBool();
    const QJsonArray retainedCookies = request.value("retainedCookies").toArray();
    if (m_id.isEmpty() || m_id.size() > 128 || !publicUrl(m_url)
        || (mode != "silent" && mode != "interactive")
        || timeout < 1000 || timeout > (m_interactive ? 180000 : 30000)
        || !profileInfo.isAbsolute() || profile.startsWith("//")
        || !profileName.match(profileInfo.fileName()).hasMatch()
        || QFileInfo {profileInfo.path()}.fileName() != "web-session"
        || m_proxy.scheme() != "http" || m_proxy.host() != "127.0.0.1" || m_proxy.port() <= 0
        || !m_proxy.userInfo().isEmpty() || !m_proxy.path().isEmpty() || m_proxy.hasQuery() || m_proxy.hasFragment()
        || m_proxyUsername != "qbutt" || m_proxyPassword.size() < 32 || m_proxyPassword.size() > 128)
    {
        fail(QStringLiteral("webview_failed"));
        return;
    }

    const bool hasConfiguredSession = request.contains("configuredSession") || request.contains("initialCookies");
    if ((hasConfiguredSession && (!profileName.match(fingerprint).hasMatch()
            || !request.value("initialCookies").isArray() || initialCookies.size() > 128))
        || (request.contains("authoritativeCookies") && (!request.value("authoritativeCookies").isBool() || !hasConfiguredSession))
        || (request.contains("retainedCookies") && (!authoritative || !request.value("retainedCookies").isArray()
            || retainedCookies.size() > 128)))
    {
        fail(QStringLiteral("webview_failed"));
        return;
    }
    const QRegularExpression cookieName {"\\A[!#$%&'*+.^_`|~0-9A-Za-z-]+\\z"};
    const QRegularExpression cookieValue {"\\A[\\x21\\x23-\\x2B\\x2D-\\x3A\\x3C-\\x5B\\x5D-\\x7E]*\\z"};
    const auto validIdentity = [this, &cookieName](const QJsonValue &value)
    {
        const QJsonObject cookie = value.toObject();
        const QString name = cookie.value("name").toString();
        return value.isObject() && name.size() <= 256 && cookieName.match(name).hasMatch()
            && cookie.value("domain").toString() == m_url.host(QUrl::EncodeUnicode) && cookie.value("path").toString() == "/"
            && cookie.value("hostOnly").isBool() && cookie.value("hostOnly").toBool();
    };
    QSet<QString> names;
    for (const QJsonValue &value : initialCookies)
    {
        const QJsonObject cookie = value.toObject();
        const QString name = cookie.value("name").toString();
        const QString content = cookie.value("value").toString();
        QString octets = content;
        if (octets.size() >= 2 && octets.startsWith('"') && octets.endsWith('"'))
            octets = octets.mid(1, octets.size() - 2);
        if (!validIdentity(value)
            || !cookie.value("value").isString() || name.size() + content.size() > 4096
            || !cookieValue.match(octets).hasMatch() || names.contains(name)
            || !cookie.value("secure").isBool()
            || !cookie.value("httpOnly").isBool()
            || (cookie.contains("expires") && (!cookie.value("expires").isDouble()
                || !std::isfinite(cookie.value("expires").toDouble())
                || cookie.value("expires").toDouble() <= QDateTime::currentSecsSinceEpoch())))
        {
            fail(QStringLiteral("webview_failed"));
            return;
        }
        names.insert(name);
    }
    QSet<QString> retained;
    for (const QJsonValue &cookie : retainedCookies)
    {
        const QString name = cookie.toObject().value("name").toString();
        if (!validIdentity(cookie) || names.contains(name))
        {
            fail(QStringLiteral("webview_failed"));
            return;
        }
        names.insert(name);
        retained.insert(name);
    }

    m_deadline.start(timeout);
    if (!QDir().mkpath(profile))
    {
        fail(QStringLiteral("webview_failed"));
        return;
    }

    WNDCLASSW definition {};
    definition.lpfnWndProc = windowProcedure;
    definition.hInstance = GetModuleHandleW(nullptr);
    definition.lpszClassName = windowClass;
    definition.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    RegisterClassW(&definition);
    const QString title = m_url.host() + " [qbutt]";
    m_window = CreateWindowExW(0, windowClass, reinterpret_cast<LPCWSTR>(title.utf16()),
        WS_OVERLAPPEDWINDOW, CW_USEDEFAULT, CW_USEDEFAULT, 960, 720, nullptr, nullptr,
        GetModuleHandleW(nullptr), this);
    if (!m_window)
    {
        fail(QStringLiteral("webview_failed"));
        return;
    }

    // Production uses only the shared Evergreen Runtime, never an Edge preview
    // channel or a caller's executable, debugging arguments or browser profile.
    const auto options = Make<CoreWebView2EnvironmentOptions>();
    ComPtr<ICoreWebView2EnvironmentOptions7> channelOptions;
    if (FAILED(options.As(&channelOptions))
        || FAILED(channelOptions->put_ReleaseChannels(COREWEBVIEW2_RELEASE_CHANNELS_STABLE)))
    {
        fail(QStringLiteral("webview_runtime_required"));
        return;
    }
    const QString arguments = "--proxy-server=" + m_proxy.toString(QUrl::FullyEncoded)
        + " --proxy-bypass-list=<-loopback> --disable-quic"
          " --force-webrtc-ip-handling-policy=disable_non_proxied_udp";
    if (FAILED(options->put_AdditionalBrowserArguments(reinterpret_cast<LPCWSTR>(arguments.utf16()))))
    {
        fail(QStringLiteral("webview_failed"));
        return;
    }
    const HRESULT result = CreateCoreWebView2EnvironmentWithOptions(nullptr,
        reinterpret_cast<LPCWSTR>(profile.utf16()), options.Get(),
        Callback<ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler>(
            [this, initialCookies, fingerprint, profile, authoritative, retained](const HRESULT status, ICoreWebView2Environment *environment) -> HRESULT
    {
        if (m_finished)
            return S_OK;
        if (FAILED(status) || !environment)
        {
            fail(status == HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)
                ? QStringLiteral("webview_runtime_required") : QStringLiteral("webview_failed"));
            return S_OK;
        }
        m_environment = environment;
        const HRESULT created = environment->CreateCoreWebView2Controller(m_window,
            Callback<ICoreWebView2CreateCoreWebView2ControllerCompletedHandler>(
                [this, initialCookies, fingerprint, profile, authoritative, retained](const HRESULT controllerStatus, ICoreWebView2Controller *controller) -> HRESULT
        {
            if (m_finished)
                return S_OK;
            if (FAILED(controllerStatus) || !controller)
            {
                fail(QStringLiteral("webview_failed"));
                return S_OK;
            }
            m_controller = controller;
            if (FAILED(controller->get_CoreWebView2(&m_view)) || !configure())
            {
                fail(QStringLiteral("webview_runtime_required"));
                return S_OK;
            }
            RECT bounds {};
            GetClientRect(m_window, &bounds);
            if (FAILED(controller->put_Bounds(bounds)) || FAILED(controller->put_IsVisible(TRUE)))
            {
                fail(QStringLiteral("webview_failed"));
                return S_OK;
            }
            prepareSession(initialCookies, fingerprint, profile, authoritative, retained);
            return S_OK;
        }).Get());
        if (FAILED(created))
            fail(QStringLiteral("webview_failed"));
        return S_OK;
    }).Get());
    if (FAILED(result))
        fail(result == HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)
            ? QStringLiteral("webview_runtime_required") : QStringLiteral("webview_failed"));
}

void WebSession::prepareSession(const QJsonArray &cookies, const QString &fingerprint, const QString &profile,
    const bool authoritative, const QSet<QString> &retained)
{
    if (fingerprint.isEmpty())
    {
        authenticateProxy();
        return;
    }
    const QString markerPath = profile + "/qbutt-configured-session";
    QFile marker {markerPath};
    bool unchanged = false;
    if (marker.exists())
    {
        if (!marker.open(QIODevice::ReadOnly))
        {
            fail(QStringLiteral("webview_failed"));
            return;
        }
        const QByteArray previous = marker.read(65);
        marker.close();
        unchanged = (previous == fingerprint.toLatin1());
        // Invalidate before clearing. An interrupted reset must be retried,
        // even if the next request switches back to the previous configuration.
        if (!unchanged && !marker.remove())
        {
            fail(QStringLiteral("webview_failed"));
            return;
        }
    }

    ComPtr<ICoreWebView2_2> view2;
    ComPtr<ICoreWebView2CookieManager> manager;
    if (FAILED(m_view.As(&view2)) || FAILED(view2->get_CookieManager(&manager)))
    {
        fail(QStringLiteral("webview_runtime_required"));
        return;
    }
    if (unchanged)
    {
        // Configured headers restore missing seeds; an active snapshot also owns
        // replacement and removal, except explicitly retained cookie identities.
        const HRESULT queried = manager->GetCookies(reinterpret_cast<LPCWSTR>(m_url.toString(QUrl::FullyEncoded).utf16()),
            Callback<ICoreWebView2GetCookiesCompletedHandler>(
                [this, cookies, manager, authoritative, retained](const HRESULT status, ICoreWebView2CookieList *list) -> HRESULT
        {
            if (m_finished)
                return S_OK;
            UINT count = 0;
            if (FAILED(status) || !list || FAILED(list->get_Count(&count)) || count > 512)
            {
                fail(QStringLiteral("webview_failed"));
                return S_OK;
            }
            QSet<QString> present;
            QSet<QString> supplied;
            for (const QJsonValue &cookie : cookies)
                supplied.insert(cookie.toObject().value("name").toString());
            for (UINT index = 0; index < count; ++index)
            {
                ComPtr<ICoreWebView2Cookie> cookie;
                LPWSTR name = nullptr;
                LPWSTR domain = nullptr;
                LPWSTR path = nullptr;
                if (FAILED(list->GetValueAtIndex(index, &cookie)) || !cookie
                    || FAILED(cookie->get_Name(&name)) || FAILED(cookie->get_Domain(&domain)) || FAILED(cookie->get_Path(&path)))
                {
                    CoTaskMemFree(name);
                    CoTaskMemFree(domain);
                    CoTaskMemFree(path);
                    fail(QStringLiteral("webview_failed"));
                    return S_OK;
                }
                const QString cookieName = consumeString(name);
                const QString cookieDomain = consumeString(domain);
                const QString cookiePath = consumeString(path);
                if (cookieDomain == m_url.host(QUrl::EncodeUnicode) && cookiePath == "/")
                {
                    present.insert(cookieName);
                    if (authoritative && !supplied.contains(cookieName) && !retained.contains(cookieName)
                        && FAILED(manager->DeleteCookie(cookie.Get())))
                    {
                        fail(QStringLiteral("webview_failed"));
                        return S_OK;
                    }
                }
            }
            QJsonArray seeds;
            for (const QJsonValue &cookie : cookies)
            {
                if (authoritative || !present.contains(cookie.toObject().value("name").toString()))
                    seeds.append(cookie);
            }
            if (!seedCookies(manager.Get(), seeds))
                fail(QStringLiteral("webview_failed"));
            else
                authenticateProxy();
            return S_OK;
        }).Get());
        if (FAILED(queried))
            fail(QStringLiteral("webview_failed"));
        return;
    }

    ComPtr<ICoreWebView2_13> view13;
    ComPtr<ICoreWebView2Profile> browserProfile;
    ComPtr<ICoreWebView2Profile2> profile2;
    if (FAILED(m_view.As(&view13)) || FAILED(view13->get_Profile(&browserProfile))
        || FAILED(browserProfile.As(&profile2)))
    {
        fail(QStringLiteral("webview_runtime_required"));
        return;
    }
    const auto data = static_cast<COREWEBVIEW2_BROWSING_DATA_KINDS>(
        COREWEBVIEW2_BROWSING_DATA_KINDS_ALL_SITE | COREWEBVIEW2_BROWSING_DATA_KINDS_DISK_CACHE);
    const HRESULT cleared = profile2->ClearBrowsingData(data,
        Callback<ICoreWebView2ClearBrowsingDataCompletedHandler>(
            [this, cookies, fingerprint, markerPath, manager](const HRESULT status) -> HRESULT
    {
        if (m_finished)
            return S_OK;
        if (FAILED(status) || !seedCookies(manager.Get(), cookies))
        {
            fail(QStringLiteral("webview_failed"));
            return S_OK;
        }
        QSaveFile marker {markerPath};
        const QByteArray contents = fingerprint.toLatin1();
        if (!marker.open(QIODevice::WriteOnly) || marker.write(contents) != contents.size() || !marker.commit())
            fail(QStringLiteral("webview_failed"));
        else
            authenticateProxy();
        return S_OK;
    }).Get());
    if (FAILED(cleared))
        fail(QStringLiteral("webview_failed"));
}

bool WebSession::seedCookies(ICoreWebView2CookieManager *manager, const QJsonArray &cookies)
{
    for (const QJsonValue &value : cookies)
    {
        const QJsonObject entry = value.toObject();
        const QString name = entry.value("name").toString();
        const QString content = entry.value("value").toString();
        const QString domain = entry.value("domain").toString();
        ComPtr<ICoreWebView2Cookie> cookie;
        if (FAILED(manager->CreateCookie(reinterpret_cast<LPCWSTR>(name.utf16()),
            reinterpret_cast<LPCWSTR>(content.utf16()), reinterpret_cast<LPCWSTR>(domain.utf16()), L"/", &cookie))
            || !cookie || FAILED(cookie->put_IsSecure(entry.value("secure").toBool()))
            || FAILED(cookie->put_IsHttpOnly(entry.value("httpOnly").toBool()))
            || FAILED(cookie->put_Expires(entry.value("expires").toDouble(-1.0)))
            || FAILED(manager->AddOrUpdateCookie(cookie.Get())))
            return false;
    }
    return true;
}

void WebSession::authenticateProxy()
{
    // Authenticate the owned proxy before navigating to a source. The gateway
    // serves this empty page without forwarding it or resolving its hostname.
    if (FAILED(m_view->Navigate(reinterpret_cast<LPCWSTR>(proxyCheckUrl.toString().utf16()))))
        fail(QStringLiteral("webview_failed"));
}

void WebSession::navigateSource()
{
    if (m_interactive)
    {
        // The first call can inherit SW_HIDE from the launching process's STARTUPINFO.
        ShowWindow(m_window, SW_SHOWNORMAL);
        ShowWindow(m_window, SW_SHOWNORMAL);
        if (!IsWindowVisible(m_window))
        {
            fail(QStringLiteral("webview_failed"));
            return;
        }
    }
    if (FAILED(m_view->Navigate(reinterpret_cast<LPCWSTR>(m_url.toString(QUrl::FullyEncoded).utf16()))))
        fail(QStringLiteral("webview_failed"));
    else
        m_poll.start();
}

bool WebSession::configure()
{
    ComPtr<ICoreWebView2Settings> settings;
    ComPtr<ICoreWebView2Settings2> settings2;
    ComPtr<ICoreWebView2Settings3> settings3;
    ComPtr<ICoreWebView2Settings4> settings4;
    ComPtr<ICoreWebView2_8> view8;
    ComPtr<ICoreWebView2_10> view10;
    ComPtr<ICoreWebView2_18> view18;
    ComPtr<ICoreWebView2_22> view22;
    if (FAILED(m_view->get_Settings(&settings)) || FAILED(settings.As(&settings2))
        || FAILED(settings.As(&settings3)) || FAILED(settings.As(&settings4))
        || FAILED(m_view.As(&view8)) || FAILED(m_view.As(&view10))
        || FAILED(m_view.As(&view18)) || FAILED(m_view.As(&view22)))
        return false;
    LPWSTR userAgent = nullptr;
    if (FAILED(settings2->get_UserAgent(&userAgent)))
        return false;
    m_userAgent = consumeString(userAgent);
    if (m_userAgent.isEmpty() || m_userAgent.size() > 2048
        || FAILED(settings->put_AreDevToolsEnabled(FALSE))
        || FAILED(settings->put_AreDefaultContextMenusEnabled(FALSE))
        || FAILED(settings->put_AreDefaultScriptDialogsEnabled(FALSE))
        || FAILED(settings->put_IsStatusBarEnabled(FALSE))
        || FAILED(settings->put_IsWebMessageEnabled(FALSE))
        || FAILED(settings->put_IsBuiltInErrorPageEnabled(FALSE))
        || FAILED(settings3->put_AreBrowserAcceleratorKeysEnabled(FALSE))
        || FAILED(settings4->put_IsGeneralAutofillEnabled(FALSE))
        || FAILED(settings4->put_IsPasswordAutosaveEnabled(FALSE))
        || FAILED(view8->put_IsMuted(TRUE)))
        return false;

    EventRegistrationToken token {};
    if (FAILED(view22->AddWebResourceRequestedFilterWithRequestSourceKinds(L"*", COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
        COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL))
        || FAILED(m_view->add_WebResourceRequested(Callback<ICoreWebView2WebResourceRequestedEventHandler>(
            [this](ICoreWebView2*, ICoreWebView2WebResourceRequestedEventArgs *args) -> HRESULT
    {
        ComPtr<ICoreWebView2WebResourceRequest> request;
        LPWSTR uri = nullptr;
        if (SUCCEEDED(args->get_Request(&request)))
            request->get_Uri(&uri);
        const QUrl address {consumeString(uri), QUrl::StrictMode};
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT context = COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL;
        args->get_ResourceContext(&context);
        const bool proxyCheck = !m_proxyAuthenticated && address == proxyCheckUrl
            && context == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT;
        if (!proxyCheck && !publicUrl(address))
        {
            ComPtr<ICoreWebView2WebResourceResponse> response;
            if (SUCCEEDED(m_environment->CreateWebResourceResponse(nullptr, 403, L"Blocked", L"Content-Length: 0", &response)))
                args->put_Response(response.Get());
            else
                fail(QStringLiteral("webview_failed"));
        }
        return S_OK;
    }).Get(), &token)))
        return false;
    if (FAILED(view18->add_LaunchingExternalUriScheme(Callback<ICoreWebView2LaunchingExternalUriSchemeEventHandler>(
        [](ICoreWebView2*, ICoreWebView2LaunchingExternalUriSchemeEventArgs *args) -> HRESULT
    {
        args->put_Cancel(TRUE);
        return S_OK;
    }).Get(), &token)))
        return false;
    if (FAILED(view10->add_BasicAuthenticationRequested(
        Callback<ICoreWebView2BasicAuthenticationRequestedEventHandler>(
            [this](ICoreWebView2*, ICoreWebView2BasicAuthenticationRequestedEventArgs *args) -> HRESULT
    {
        LPWSTR uri = nullptr;
        LPWSTR challenge = nullptr;
        args->get_Uri(&uri);
        args->get_Challenge(&challenge);
        const QUrl address {consumeString(uri), QUrl::StrictMode};
        const QString authentication = consumeString(challenge);
        // Microsoft documents proxy challenges with the proxy URI. Never
        // supply the ephemeral proxy secret to a source's HTTP auth prompt.
        if (address.scheme() != m_proxy.scheme() || address.host() != m_proxy.host()
            || address.port() != m_proxy.port() || !address.userInfo().isEmpty()
            || (!address.path().isEmpty() && address.path() != "/") || address.hasQuery() || address.hasFragment()
            || authentication != "Basic realm=\"qbutt\"")
        {
            args->put_Cancel(TRUE);
            return S_OK;
        }
        ComPtr<ICoreWebView2BasicAuthenticationResponse> response;
        if (SUCCEEDED(args->get_Response(&response)))
        {
            response->put_UserName(reinterpret_cast<LPCWSTR>(m_proxyUsername.utf16()));
            response->put_Password(reinterpret_cast<LPCWSTR>(m_proxyPassword.utf16()));
        }
        else
            args->put_Cancel(TRUE);
        return S_OK;
    }).Get(), &token)))
        return false;

    if (FAILED(m_view->add_NavigationStarting(Callback<ICoreWebView2NavigationStartingEventHandler>(
        [this](ICoreWebView2*, ICoreWebView2NavigationStartingEventArgs *args) -> HRESULT
    {
        args->get_NavigationId(&m_navigationId);
        LPWSTR uri = nullptr;
        args->get_Uri(&uri);
        const QUrl address {consumeString(uri), QUrl::StrictMode};
        if ((!m_proxyAuthenticated && address != proxyCheckUrl)
            || (m_proxyAuthenticated && !publicUrl(address)))
        {
            args->put_Cancel(TRUE);
            fail(QStringLiteral("webview_failed"));
        }
        m_loaded = false;
        m_cleanPolls = 0;
        ++m_navigationGeneration;
        return S_OK;
    }).Get(), &token)))
        return false;
    if (FAILED(m_view->add_FrameNavigationStarting(Callback<ICoreWebView2NavigationStartingEventHandler>(
        [](ICoreWebView2*, ICoreWebView2NavigationStartingEventArgs *args) -> HRESULT
    {
        LPWSTR uri = nullptr;
        args->get_Uri(&uri);
        const QUrl address {consumeString(uri), QUrl::StrictMode};
        if (address != QUrl {"about:blank"} && !publicUrl(address))
            args->put_Cancel(TRUE);
        return S_OK;
    }).Get(), &token)))
        return false;
    if (FAILED(m_view->add_NavigationCompleted(Callback<ICoreWebView2NavigationCompletedEventHandler>(
        [this](ICoreWebView2*, ICoreWebView2NavigationCompletedEventArgs *args) -> HRESULT
    {
        UINT64 navigationId = 0;
        if (FAILED(args->get_NavigationId(&navigationId)) || navigationId != m_navigationId)
            return S_OK;
        BOOL success = FALSE;
        args->get_IsSuccess(&success);
        ComPtr<ICoreWebView2NavigationCompletedEventArgs2> details;
        m_httpStatus = 0;
        if (SUCCEEDED(args->QueryInterface(IID_PPV_ARGS(&details))))
            details->get_HttpStatusCode(&m_httpStatus);
        COREWEBVIEW2_WEB_ERROR_STATUS error = COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN;
        args->get_WebErrorStatus(&error);
        if (!m_proxyAuthenticated)
        {
            // WebView reports the challenged navigation before requesting
            // credentials and starting its authenticated navigation.
            if (m_httpStatus == 407 || error == COREWEBVIEW2_WEB_ERROR_STATUS_VALID_PROXY_AUTHENTICATION_REQUIRED)
                return S_OK;
            if (!success || m_httpStatus != 200)
                fail(QStringLiteral("webview_failed"));
            else
            {
                m_proxyAuthenticated = true;
                QTimer::singleShot(0, this, [this]
                {
                    if (!m_finished)
                        navigateSource();
                });
            }
            return S_OK;
        }
        if (!success && m_httpStatus < 400 && error != COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED)
            fail(QStringLiteral("webview_failed"));
        else if (success || m_httpStatus >= 400)
            m_loaded = true;
        return S_OK;
    }).Get(), &token)))
        return false;
    if (FAILED(m_view->add_NewWindowRequested(Callback<ICoreWebView2NewWindowRequestedEventHandler>(
        [](ICoreWebView2*, ICoreWebView2NewWindowRequestedEventArgs *args) -> HRESULT
    {
        args->put_Handled(TRUE);
        return S_OK;
    }).Get(), &token)))
        return false;
    if (FAILED(m_view->add_PermissionRequested(Callback<ICoreWebView2PermissionRequestedEventHandler>(
        [](ICoreWebView2*, ICoreWebView2PermissionRequestedEventArgs *args) -> HRESULT
    {
        args->put_State(COREWEBVIEW2_PERMISSION_STATE_DENY);
        return S_OK;
    }).Get(), &token)))
        return false;
    ComPtr<ICoreWebView2_4> view4;
    if (FAILED(m_view.As(&view4)))
        return false;
    if (FAILED(view4->add_DownloadStarting(Callback<ICoreWebView2DownloadStartingEventHandler>(
        [](ICoreWebView2*, ICoreWebView2DownloadStartingEventArgs *args) -> HRESULT
    {
        args->put_Cancel(TRUE);
        args->put_Handled(TRUE);
        return S_OK;
    }).Get(), &token)))
        return false;
    if (FAILED(m_view->add_ProcessFailed(Callback<ICoreWebView2ProcessFailedEventHandler>(
        [this](ICoreWebView2*, ICoreWebView2ProcessFailedEventArgs*) -> HRESULT
    {
        fail(QStringLiteral("webview_failed"));
        return S_OK;
    }).Get(), &token)))
        return false;
    return true;
}

void WebSession::pollDocument()
{
    if (m_finished || !m_loaded || m_polling)
        return;
    m_polling = true;
    const quint64 generation = m_navigationGeneration;
    const HRESULT result = m_view->ExecuteScript(documentState,
        Callback<ICoreWebView2ExecuteScriptCompletedHandler>([this, generation](const HRESULT status, LPCWSTR json) -> HRESULT
    {
        m_polling = false;
        if (m_finished || !m_loaded || generation != m_navigationGeneration)
            return S_OK;
        if (FAILED(status) || !json)
        {
            fail(QStringLiteral("webview_failed"));
            return S_OK;
        }
        const QByteArray data = QString::fromWCharArray(json).toUtf8();
        if (data.size() > 16 * 1024)
        {
            fail(QStringLiteral("webview_failed"));
            return S_OK;
        }
        const QJsonObject state = QJsonDocument::fromJson(data).object();
        const QUrl address {state.value("url").toString(), QUrl::StrictMode};
        if (!publicUrl(address))
        {
            fail(QStringLiteral("webview_failed"));
            return S_OK;
        }
        if (state.value("challenge").toBool())
        {
            m_sawChallenge = true;
            m_cleanPolls = 0;
        }
        else if (state.value("ready").toBool() && ++m_cleanPolls >= 2)
        {
            if (m_httpStatus >= 400)
                fail(m_httpStatus == 401 || m_httpStatus == 403
                    ? QStringLiteral("browser_check_required") : QStringLiteral("webview_failed"));
            else
                collectSession(address);
        }
        return S_OK;
    }).Get());
    if (FAILED(result))
        fail(QStringLiteral("webview_failed"));
}

void WebSession::collectSession(const QUrl &url)
{
    m_poll.stop();
    const quint64 generation = m_navigationGeneration;
    ComPtr<ICoreWebView2_2> view2;
    ComPtr<ICoreWebView2CookieManager> manager;
    if (FAILED(m_view.As(&view2)) || FAILED(view2->get_CookieManager(&manager)))
    {
        fail(QStringLiteral("webview_runtime_required"));
        return;
    }
    const HRESULT result = manager->GetCookies(reinterpret_cast<LPCWSTR>(url.toString(QUrl::FullyEncoded).utf16()),
        Callback<ICoreWebView2GetCookiesCompletedHandler>(
            [this, url, generation](const HRESULT status, ICoreWebView2CookieList *list) -> HRESULT
    {
        if (m_finished)
            return S_OK;
        if (generation != m_navigationGeneration)
        {
            m_poll.start();
            return S_OK;
        }
        UINT count = 0;
        if (FAILED(status) || !list || FAILED(list->get_Count(&count)) || count > 512)
        {
            fail(QStringLiteral("webview_failed"));
            return S_OK;
        }
        QJsonArray cookies;
        for (UINT index = 0; index < count; ++index)
        {
            ComPtr<ICoreWebView2Cookie> cookie;
            if (FAILED(list->GetValueAtIndex(index, &cookie)))
            {
                fail(QStringLiteral("webview_failed"));
                return S_OK;
            }
            LPWSTR name = nullptr;
            LPWSTR value = nullptr;
            LPWSTR domain = nullptr;
            LPWSTR path = nullptr;
            BOOL secure = FALSE;
            BOOL httpOnly = FALSE;
            BOOL session = FALSE;
            double expires = 0;
            const bool valid = SUCCEEDED(cookie->get_Name(&name))
                && SUCCEEDED(cookie->get_Value(&value))
                && SUCCEEDED(cookie->get_Domain(&domain))
                && SUCCEEDED(cookie->get_Path(&path))
                && SUCCEEDED(cookie->get_IsSecure(&secure))
                && SUCCEEDED(cookie->get_IsHttpOnly(&httpOnly))
                && SUCCEEDED(cookie->get_IsSession(&session))
                && SUCCEEDED(cookie->get_Expires(&expires));
            if (!valid)
            {
                CoTaskMemFree(name);
                CoTaskMemFree(value);
                CoTaskMemFree(domain);
                CoTaskMemFree(path);
                fail(QStringLiteral("webview_failed"));
                return S_OK;
            }
            const QString cookieDomain = consumeString(domain);
            QJsonObject entry {
                {"name", consumeString(name)},
                {"value", consumeString(value)},
                {"domain", cookieDomain},
                {"path", consumeString(path)},
                {"secure", static_cast<bool>(secure)},
                {"httpOnly", static_cast<bool>(httpOnly)},
                {"hostOnly", !cookieDomain.startsWith('.')}
            };
            if (!session && std::isfinite(expires) && expires > 0)
                entry.insert("expires", expires);
            cookies.append(entry);
        }
        const QJsonObject session {{"url", url.toString(QUrl::FullyEncoded)}, {"userAgent", m_userAgent}, {"cookies", cookies}};
        finish({{"session", session}});
        return S_OK;
    }).Get());
    if (FAILED(result))
        fail(QStringLiteral("webview_failed"));
}

void WebSession::cancel()
{
    fail(QStringLiteral("canceled"));
}

void WebSession::fail(const QString &code)
{
    finish({{"error", QJsonObject {{"code", code}, {"url", m_url.toString(QUrl::FullyEncoded)}}}});
}

void WebSession::finish(const QJsonObject &result)
{
    if (m_finished)
        return;
    m_finished = true;
    m_deadline.stop();
    m_poll.stop();
    QTimer::singleShot(0, this, [this, result]
    {
        close();
        QJsonObject response = result;
        response.insert("id", m_id);
        if (QJsonDocument {response}.toJson(QJsonDocument::Compact).size() > maxResponseBytes)
            response = {{"id", m_id}, {"error", QJsonObject {{"code", "webview_failed"}, {"url", m_url.toString(QUrl::FullyEncoded)}}}};
        m_completed(response);
    });
}

void WebSession::close()
{
    m_deadline.stop();
    m_poll.stop();
    if (m_controller)
        m_controller->Close();
    m_view.Reset();
    m_controller.Reset();
    m_environment.Reset();
    if (m_window)
    {
        SetWindowLongPtrW(m_window, GWLP_USERDATA, 0);
        DestroyWindow(m_window);
        m_window = nullptr;
    }
}

bool WebSession::publicUrl(const QUrl &url)
{
    if (!url.isValid() || (url.scheme() != "https" && url.scheme() != "http")
        || url.host().isEmpty() || url.host() == proxyCheckUrl.host()
        || !url.userInfo().isEmpty() || url.port(443) <= 0)
        return false;
    const QHostAddress address {url.host()};
    if (address.isNull())
        return true;
    bool isV4 = false;
    const quint32 ip = address.toIPv4Address(&isV4);
    if (isV4)
    {
        const quint32 first = ip >> 24;
        const quint32 second = (ip >> 16) & 255;
        return first != 0 && first != 10 && first != 127 && first < 224
            && !(first == 169 && second == 254)
            && !(first == 172 && second >= 16 && second <= 31)
            && !(first == 192 && second == 168)
            && !(first == 100 && second >= 64 && second <= 127);
    }
    return (address.toIPv6Address()[0] & 0xE0) == 0x20;
}

LRESULT CALLBACK WebSession::windowProcedure(HWND window, const UINT message, const WPARAM wParam, const LPARAM lParam)
{
    if (message == WM_NCCREATE)
    {
        const auto creation = reinterpret_cast<CREATESTRUCTW*>(lParam);
        SetWindowLongPtrW(window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(creation->lpCreateParams));
    }
    const auto session = reinterpret_cast<WebSession*>(GetWindowLongPtrW(window, GWLP_USERDATA));
    if (session)
    {
        if (message == WM_CLOSE || (message == WM_KEYDOWN && wParam == VK_ESCAPE))
        {
            session->cancel();
            return 0;
        }
        if (message == WM_SIZE && session->m_controller)
        {
            RECT bounds {};
            GetClientRect(window, &bounds);
            session->m_controller->put_Bounds(bounds);
        }
    }
    return DefWindowProcW(window, message, wParam, lParam);
}
