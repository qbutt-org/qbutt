// SPDX-License-Identifier: GPL-2.0-or-later
#include "browsercapture.h"

#include <QJsonDocument>
#include <QJsonObject>
#include <QRegularExpression>
#include <QUrlQuery>

#include "base/global.h"

namespace
{
    bool publicPage(const QUrl &page)
    {
        if (!page.isValid() || (page.scheme() != u"https" && page.scheme() != u"http")
            || page.host().isEmpty() || !page.userInfo().isEmpty() || page.hasFragment())
        {
            return false;
        }

        static const QRegularExpression sensitiveKey(
            u"(?:token|passkey|password|secret|auth|cookie|signature|api.?key)|^(?:key|sig|expires|policy|x-amz-.+|x-goog-.+)$"_s,
            QRegularExpression::CaseInsensitiveOption);
        const QUrlQuery query(page);
        for (const auto &item : query.queryItems())
        {
            if (sensitiveKey.match(item.first).hasMatch())
                return false;
        }
        return true;
    }
}

Content::BrowserCapture::BrowserCapture(QObject *parent)
    : QObject(parent)
{
}

void Content::BrowserCapture::open(const QUrl &url)
{
    cancel();
    static const QRegularExpression payloadPattern(u"^d=([A-Za-z0-9_-]+)$"_s);
    static const QRegularExpression capturePattern(
        u"^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$"_s,
        QRegularExpression::CaseInsensitiveOption);

    const auto payloadMatch = payloadPattern.match(url.query(QUrl::FullyEncoded));
    if (!url.isValid() || url.toString(QUrl::FullyEncoded).size() > 24 * 1024
        || url.scheme() != u"qbutt" || url.host() != u"capture"
        || !url.path().isEmpty() || url.port() != -1 || !url.userInfo().isEmpty()
        || url.hasFragment() || !payloadMatch.hasMatch())
    {
        emit error(tr("This qbutt link is invalid or unsupported."));
        return;
    }

    const QJsonDocument document = QJsonDocument::fromJson(
        QByteArray::fromBase64(payloadMatch.captured(1).toLatin1(), QByteArray::Base64UrlEncoding));
    if (!document.isObject())
    {
        emit error(tr("This qbutt link is invalid or unsupported."));
        return;
    }

    const QJsonObject capture = document.object();
    const QString action = capture.value(u"action"_s).toString();
    const QString captureId = capture.value(u"captureId"_s).toString();
    const QString pageText = capture.value(u"pageUrl"_s).toString();
    const QString titleText = capture.value(u"title"_s).toString();
    const QUrl page(pageText, QUrl::StrictMode);
    if (capture.value(u"v"_s).toInt() != 1
        || (action != u"watch" && action != u"download")
        || pageText.isEmpty() || pageText.size() > 4096 || !publicPage(page)
        || !capturePattern.match(captureId).hasMatch()
        || !capture.value(u"title"_s).isString() || titleText.size() > 512)
    {
        emit error(tr("This qcatch capture is incomplete or unsupported. Choose the media again in qcatch."));
        return;
    }

    m_page = page;
    m_captureId = captureId;
    m_title = titleText;
    for (qsizetype index = 0; index < m_title.size(); ++index)
    {
        const ushort character = m_title.at(index).unicode();
        if (character < 32 || character == 127)
            m_title[index] = QLatin1Char(' ');
    }
    m_title = m_title.trimmed();
    if (m_title.isEmpty())
        m_title = page.host();
    m_action = action == u"watch" ? Action::Watch : Action::Download;
    dispatch();
}

void Content::BrowserCapture::retry()
{
    if (m_action != Action::None && !m_pending)
        dispatch();
}

void Content::BrowserCapture::cancel()
{
    if (m_pending)
    {
        if (m_action == Action::Watch)
            emit watchCancelled();
        else if (m_action == Action::Download)
            emit downloadCancelled();
    }
    m_pending = false;
    m_action = Action::None;
    m_page = {};
    m_title.clear();
    m_captureId.clear();
    emit preparationChanged({});
}

bool Content::BrowserCapture::requestPlaybackDownload(const QString &identity)
{
    if (m_pending || m_playbackCaptureId.isEmpty()
        || identity != u"capture:"_s + m_playbackCaptureId)
    {
        return false;
    }
    m_page = m_playbackPage;
    m_title = m_playbackTitle;
    m_captureId = m_playbackCaptureId;
    m_action = Action::Download;
    dispatch();
    return true;
}

void Content::BrowserCapture::dispatch()
{
    m_pending = true;
    emit preparationChanged(m_action == Action::Watch
        ? tr("Opening qcatch video…") : tr("Preparing download…"));
    if (m_action == Action::Watch)
        emit watchRequested(m_page, m_title, m_captureId);
    else
        emit downloadRequested(m_page, m_title, m_captureId);
}

void Content::BrowserCapture::finishDownload(const QString &captureId, const QString &reason)
{
    finish(captureId, reason, Action::Download);
}

void Content::BrowserCapture::finishWatch(const QString &captureId, const QString &reason)
{
    finish(captureId, reason, Action::Watch);
}

void Content::BrowserCapture::finish(const QString &captureId, const QString &reason, const Action action)
{
    if (!m_pending || m_action != action || captureId != m_captureId)
        return;
    m_pending = false;
    emit preparationChanged({});
    if (reason == u"browser_required")
    {
        emit browserRequired();
        return;
    }
    m_action = Action::None;
    if (reason.isEmpty())
    {
        if (action == Action::Watch)
        {
            m_playbackPage = m_page;
            m_playbackTitle = m_title;
            m_playbackCaptureId = m_captureId;
        }
        emit completed();
    }
    else if (reason == u"unsupported")
    {
        emit error(action == Action::Download
            ? tr("This video cannot be saved as one MP4 file.")
            : tr("This video cannot be opened."));
    }
    else if (reason == u"capture")
    {
        emit error(tr("This qcatch selection expired. Choose the video again in qcatch."));
    }
    else
    {
        emit error(action == Action::Download
            ? tr("The download could not be queued.")
            : tr("Could not open this qcatch video."));
    }
}
