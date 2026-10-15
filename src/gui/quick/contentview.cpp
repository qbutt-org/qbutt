// SPDX-License-Identifier: GPL-2.0-or-later
#include "contentview.h"

#include <QEvent>
#include <QDesktopServices>
#include <QFileInfo>
#include <QQmlEngine>
#include <QQuickItem>

#include "base/media/runtime.h"
#include "readerimageprovider.h"
#include "qbutt_quick/src/controller.cxxqt.h"

namespace
{
    Qbutt::ContentError downloadRuntimeError(const Media::Runtime::Error error)
    {
        switch (error)
        {
        case Media::Runtime::Error::None:
            return Qbutt::ContentError::None;
        case Media::Runtime::Error::Missing:
            return Qbutt::ContentError::MediaRuntimeMissing;
        case Media::Runtime::Error::CouldNotStart:
            return Qbutt::ContentError::MediaRuntimeCouldNotStart;
        case Media::Runtime::Error::InvalidResponse:
            return Qbutt::ContentError::MediaRuntimeInvalidResponse;
        case Media::Runtime::Error::IncompatibleVersion:
            return Qbutt::ContentError::MediaRuntimeIncompatibleVersion;
        case Media::Runtime::Error::IncompleteInstallation:
            return Qbutt::ContentError::MediaRuntimeIncompleteInstallation;
        case Media::Runtime::Error::Timeout:
            return Qbutt::ContentError::MediaRuntimeTimeout;
        }
        Q_UNREACHABLE();
    }
}

Content::ContentView::ContentView(const QString &profileDir, QWidget *parent)
    : QQuickWidget(parent)
    , m_controller {new Qbutt::ContentController}
{
    setObjectName(QStringLiteral("contentView"));
    m_readerImages = new ReaderImageProvider;
    engine()->addImageProvider(QStringLiteral("reader"), m_readerImages);
    m_controller->setParent(this);
    m_controller->initLibrary(profileDir);
    m_controller->setObjectName(QStringLiteral("contentController"));
    connect(m_controller, &Qbutt::ContentController::torrentReady, this, &ContentView::torrentReady);
    connect(m_controller, &Qbutt::ContentController::torrentMetadataReady, this, &ContentView::torrentMetadataReady);
    connect(m_controller, &Qbutt::ContentController::videoReady, this, &ContentView::videoReady);
    connect(m_controller, &Qbutt::ContentController::browserReply, this, &ContentView::browserReply);
    connect(m_controller, &Qbutt::ContentController::browserChoiceRequested,
        this, &ContentView::browserChoiceRequested);
    connect(m_controller, &Qbutt::ContentController::downloadQueued, this, &ContentView::downloadQueued);
    connect(m_controller, &Qbutt::ContentController::playbackDownloadFinished, this, &ContentView::playbackDownloadFinished);
    connect(m_controller, &Qbutt::ContentController::mediaToolsRequested, this, [this](const quint64 requestId)
    {
        const quint64 runtimeId = Media::Runtime::instance()->request(this,
            [this, requestId](const Media::Runtime::Result &result)
        {
            if (!m_mediaRequests.remove(requestId))
                return;

            const Qbutt::ContentError error = downloadRuntimeError(result.error);
            m_controller->mediaToolsReady(requestId, result.info.ffmpeg, result.info.ffprobe, error);
        });
        m_mediaRequests.insert(requestId, runtimeId);
    });
    connect(m_controller, &Qbutt::ContentController::mediaToolsCancelled, this, [this](const quint64 requestId)
    {
        const quint64 runtimeId = m_mediaRequests.take(requestId);
        if (runtimeId != 0)
            Media::Runtime::instance()->cancel(runtimeId);
    });
    connect(m_controller, &Qbutt::ContentController::capturedDownloadFinished,
        this, &ContentView::capturedDownloadFinished);
    connect(m_controller, &Qbutt::ContentController::capturedWatchFinished,
        this, &ContentView::capturedWatchFinished);
    connect(m_controller, &Qbutt::ContentController::readerChanged, this, &ContentView::readerActiveChanged);
    connect(m_controller, &Qbutt::ContentController::readerAssetsCleared,
        this, [this](const quint64 generation) { m_readerImages->clear(generation); });
    connect(m_controller, &Qbutt::ContentController::readerAsset,
        this, [this](const quint64 generation, const QString &id, const QByteArray &bytes)
        { m_readerImages->add(generation, id, bytes); });
    connect(m_controller, &Qbutt::ContentController::sharedLinkFailed, this, &ContentView::sharedLinkFailed);
    connect(m_controller, &Qbutt::ContentController::searchStateChanged, this, [this]
    {
        const auto state = m_controller->getSearchState();
        if ((state == Qbutt::Search::SearchState::Succeeded) || (state == Qbutt::Search::SearchState::Failed))
            emit searchFinished(state == Qbutt::Search::SearchState::Failed);
    });

    setResizeMode(QQuickWidget::SizeRootObjectToView);
    setInitialProperties({{QStringLiteral("backend"), QVariant::fromValue(m_controller)}});
    loadFromModule(QStringLiteral("Qbutt.Views"), QStringLiteral("SearchPage"));
    if (rootObject())
    {
        connect(rootObject(), SIGNAL(browserSetupReady()), this, SIGNAL(browserSetupReady()));
        connect(rootObject(), SIGNAL(browserSetupDismissed()), this, SIGNAL(browserSetupDismissed()));
    }
}

Content::ContentView::~ContentView()
{
    for (const quint64 runtimeId : m_mediaRequests)
        Media::Runtime::instance()->cancel(runtimeId);
    m_controller->closeBrowserRelay();
}

void Content::ContentView::openLocalBookPicker()
{
    if (rootObject())
        QMetaObject::invokeMethod(rootObject(), "openLocalBookPicker");
}

QQuickWidget *Content::ContentView::createDownloadsView(QWidget *parent)
{
    auto *view = new QQuickWidget(parent);
    view->setObjectName(QStringLiteral("downloadsView"));
    view->installEventFilter(this);
    view->setResizeMode(QQuickWidget::SizeRootObjectToView);
    view->setInitialProperties({{QStringLiteral("backend"), QVariant::fromValue(m_controller)}});
    view->loadFromModule(QStringLiteral("Qbutt.Views"), QStringLiteral("DownloadsPage"));
    if (view->rootObject())
    {
        connect(view->rootObject(), SIGNAL(openFolderRequested(QString)),
            this, SLOT(openDownloadedFolder(QString)));
    }
    return view;
}

QQuickWidget *Content::ContentView::createSourcesView(QWidget *parent)
{
    auto *view = new QQuickWidget(parent);
    view->setObjectName(QStringLiteral("sourcesView"));
    view->installEventFilter(this);
    view->setResizeMode(QQuickWidget::SizeRootObjectToView);
    view->setInitialProperties({{QStringLiteral("backend"), QVariant::fromValue(m_controller)}});
    view->loadFromModule(QStringLiteral("Qbutt.Views"), QStringLiteral("SourcesPage"));
    return view;
}

void Content::ContentView::initDownloads(const QString &destination)
{
    m_controller->initDownloads(destination);
}

bool Content::ContentView::canDownloadPlayback(const quint64 launchToken) const
{
    return m_controller->canDownloadPlayback(launchToken);
}

bool Content::ContentView::downloadPlayback(const quint64 launchToken)
{
    return m_controller->downloadPlayback(launchToken);
}

bool Content::ContentView::downloadCapture(const QUrl &page, const QString &title, const QString &captureId)
{
    return m_controller->downloadCaptured(page.toString(QUrl::FullyEncoded), title, captureId);
}

void Content::ContentView::cancelCaptureDownload()
{
    m_controller->cancelCapturedDownload();
}

bool Content::ContentView::watchCapture(const QUrl &page, const QString &title, const QString &captureId)
{
    return m_controller->watchCaptured(page.toString(QUrl::FullyEncoded), title, captureId);
}

void Content::ContentView::cancelCaptureWatch()
{
    m_controller->cancelCapturedWatch();
}

void Content::ContentView::openDownloadedFolder(const QString &path)
{
    const QFileInfo file(path);
    if (file.isAbsolute() && file.isFile())
        QDesktopServices::openUrl(QUrl::fromLocalFile(file.absolutePath()));
}

void Content::ContentView::initializeBrowser()
{
    m_controller->initializeBrowser();
}

void Content::ContentView::setBrowserChoice(const QString &choice)
{
    m_controller->setBrowserChoice(choice);
}

void Content::ContentView::openBrowserSettingsForCapture()
{
    if (rootObject())
        QMetaObject::invokeMethod(rootObject(), "openBrowserSettingsForCapture");
}

void Content::ContentView::closeBrowserSettingsForCapture()
{
    if (rootObject())
        QMetaObject::invokeMethod(rootObject(), "closeBrowserSettingsForCapture");
}

void Content::ContentView::relayBrowserRequest(const quint64 generation, const quint64 id, const QByteArray &request)
{
    m_controller->relayBrowserRequest(generation, id, request);
}

void Content::ContentView::cancelBrowserRequest(const quint64 generation, const quint64 id)
{
    m_controller->cancelBrowserRequest(generation, id);
}

void Content::ContentView::closeBrowserSession(const quint64 generation)
{
    m_controller->closeBrowserSession(generation);
}

void Content::ContentView::openSharedLink(const QUrl &url)
{
    m_controller->openSharedLink(url.toString(QUrl::FullyEncoded));
}

void Content::ContentView::giveFocusToSearchInput()
{
    if (!rootObject())
        return;

    if (auto *query = rootObject()->findChild<QQuickItem *>(QStringLiteral("contentQuery")))
    {
        query->forceActiveFocus(Qt::ShortcutFocusReason);
        QMetaObject::invokeMethod(query, "selectAll");
    }
}

bool Content::ContentView::isReading() const
{
    return m_controller->reader_open();
}

bool Content::ContentView::confirmPlayback(const quint64 launchToken)
{
    return m_controller->confirmPlayback(launchToken);
}

bool Content::ContentView::finishPlayback(const quint64 launchToken)
{
    return m_controller->finishPlayback(launchToken);
}

void Content::ContentView::changeEvent(QEvent *event)
{
    QQuickWidget::changeEvent(event);
    if (event->type() == QEvent::LanguageChange)
    {
        m_controller->retranslate();
        engine()->retranslate();
    }
}

bool Content::ContentView::eventFilter(QObject *watched, QEvent *event)
{
    if (event->type() == QEvent::LanguageChange)
    {
        if (auto *view = qobject_cast<QQuickWidget *>(watched))
            view->engine()->retranslate();
    }
    return QQuickWidget::eventFilter(watched, event);
}
