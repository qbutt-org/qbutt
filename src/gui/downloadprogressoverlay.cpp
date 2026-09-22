// SPDX-License-Identifier: GPL-2.0-or-later
#include "downloadprogressoverlay.h"

#include <algorithm>

#include <QGuiApplication>
#include <QPainter>
#include <QScreen>

#include "base/bittorrent/session.h"
#include "base/bittorrent/torrent.h"
#include "base/global.h"
#include "base/preferences.h"

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
    const QRect available = screen->availableGeometry();
    constexpr int HEIGHT = 3;
    setGeometry(available.left(), available.bottom() - HEIGHT + 1, available.width(), HEIGHT);
    setVisible(true);
    update();
}

void DownloadProgressOverlay::paintEvent(QPaintEvent *)
{
    QPainter painter(this);
    painter.fillRect(QRectF(0, 0, width() * m_progress, height()), palette().brush(QPalette::Active, QPalette::Link));
}
