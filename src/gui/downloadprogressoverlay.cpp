// SPDX-License-Identifier: GPL-2.0-or-later
#include "downloadprogressoverlay.h"

#include <algorithm>

#ifdef Q_OS_WIN
#include <windows.h>
#endif

#include <QGuiApplication>
#include <QPainter>
#include <QScreen>
#include <QtMath>

#include "base/bittorrent/session.h"
#include "base/bittorrent/torrent.h"
#include "base/global.h"
#include "base/preferences.h"

namespace
{
    constexpr int OVERLAY_HEIGHT = 3;

    QRect placementArea(QScreen *screen)
    {
#ifdef Q_OS_WIN
        if (QGuiApplication::platformName() == u"windows"_s)
        {
            QRect area = screen->geometry();
            const HWND taskbar = FindWindowW(L"Shell_TrayWnd", nullptr);
            if (!taskbar || !IsWindowVisible(taskbar))
                return area;

            MONITORINFO monitor {sizeof(MONITORINFO)};
            const POINT primaryPoint {0, 0};
            if (!GetMonitorInfoW(MonitorFromPoint(primaryPoint, MONITOR_DEFAULTTOPRIMARY), &monitor))
                return area;

            RECT window;
            if (!GetWindowRect(taskbar, &window))
                return area;
            const RECT visible {std::max(window.left, monitor.rcMonitor.left),
                std::max(window.top, monitor.rcMonitor.top),
                std::min(window.right, monitor.rcMonitor.right),
                std::min(window.bottom, monitor.rcMonitor.bottom)};
            const int width = visible.right - visible.left;
            const int height = visible.bottom - visible.top;
            if (std::min(width, height) < qCeil(OVERLAY_HEIGHT * screen->devicePixelRatio()))
            {
                return area;
            }

            const auto taskbarAt = [taskbar](const POINT point)
            {
                const HWND windowAtPoint = WindowFromPoint(point);
                return windowAtPoint && (GetAncestor(windowAtPoint, GA_ROOT) == taskbar);
            };
            bool exposed = false;
            for (int quarter = 1; quarter <= 3; ++quarter)
            {
                const POINT point {(width >= height)
                        ? visible.left + width * quarter / 4 : visible.left + width / 2,
                    (width >= height)
                        ? visible.top + height / 2 : visible.top + height * quarter / 4};
                exposed |= taskbarAt(point);
            }
            if (!exposed)
                return area;

            const int monitorWidth = monitor.rcMonitor.right - monitor.rcMonitor.left;
            const int monitorHeight = monitor.rcMonitor.bottom - monitor.rcMonitor.top;
            if ((monitorWidth <= 0) || (monitorHeight <= 0))
                return area;
            const auto qtX = [&](const int x)
            {
                return area.left() + qRound(qreal(x - monitor.rcMonitor.left) * area.width() / monitorWidth);
            };
            const auto qtY = [&](const int y)
            {
                return area.top() + qRound(qreal(y - monitor.rcMonitor.top) * area.height() / monitorHeight);
            };
            if ((width >= height) && (visible.bottom == monitor.rcMonitor.bottom))
                area.setBottom(qtY(visible.top) - 1);
            else if ((height > width) && (visible.left == monitor.rcMonitor.left))
                area.setLeft(qtX(visible.right));
            else if ((height > width) && (visible.right == monitor.rcMonitor.right))
                area.setRight(qtX(visible.left) - 1);
            return area;
        }
#endif
        return screen->availableGeometry();
    }
}

DownloadProgressOverlay::DownloadProgressOverlay()
    : QWidget(nullptr, Qt::Tool | Qt::FramelessWindowHint | Qt::WindowStaysOnTopHint
        | Qt::WindowTransparentForInput | Qt::WindowDoesNotAcceptFocus)
{
    setObjectName(u"downloadProgressOverlay"_s);
    setAttribute(Qt::WA_ShowWithoutActivating);
    setAttribute(Qt::WA_TransparentForMouseEvents);
    setAttribute(Qt::WA_TranslucentBackground);
    setFocusPolicy(Qt::NoFocus);

    const auto *session = BitTorrent::Session::instance();
    connect(session, &BitTorrent::Session::statsUpdated, this, &DownloadProgressOverlay::refresh);
    connect(session, &BitTorrent::Session::paused, this, &DownloadProgressOverlay::refresh);
    connect(session, &BitTorrent::Session::resumed, this, &DownloadProgressOverlay::refresh);
    connect(Preferences::instance(), &Preferences::changed, this, &DownloadProgressOverlay::refresh);
    connect(qGuiApp, &QGuiApplication::primaryScreenChanged, this, &DownloadProgressOverlay::refresh);
    refresh();
}

qreal DownloadProgressOverlay::progress() const
{
    return m_progress;
}

void DownloadProgressOverlay::refresh()
{
    const auto *session = BitTorrent::Session::instance();
    QScreen *const screen = QGuiApplication::primaryScreen();
    m_progress = 0;
    if (!Preferences::instance()->isDownloadProgressOverlayEnabled() || session->isPaused() || !screen)
    {
        hide();
        return;
    }

    qreal wanted = 0;
    qreal completed = 0;
    for (const auto *torrent : session->torrents())
    {
        if (!torrent->isDownloading() || torrent->isStopped() || torrent->isQueued() || torrent->isChecking())
            continue;

        const qlonglong size = torrent->wantedSize();
        if (size <= 0)
            continue;

        wanted += size;
        completed += std::clamp(torrent->completedSize(), 0LL, size);
    }

    if (wanted <= 0)
    {
        hide();
        return;
    }

    m_progress = completed / wanted;
    const QRect area = placementArea(screen);
    setGeometry(area.left(), area.bottom() - OVERLAY_HEIGHT + 1, area.width(), OVERLAY_HEIGHT);
    setVisible(true);
    update();
}

void DownloadProgressOverlay::paintEvent(QPaintEvent *)
{
    QPainter painter(this);
    painter.fillRect(QRectF(0, 0, width() * m_progress, height()), palette().brush(QPalette::Active, QPalette::Link));
}
