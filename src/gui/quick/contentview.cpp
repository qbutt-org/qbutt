// SPDX-License-Identifier: GPL-2.0-or-later
#include "contentview.h"

#include <QEvent>
#include <QBuffer>
#include <QDesktopServices>
#include <QHash>
#include <QImageReader>
#include <QFileInfo>
#include <QQmlEngine>
#include <QQuickImageProvider>
#include <QQuickItem>
#include <QReadWriteLock>

#include "qbutt_quick/src/controller.cxxqt.h"

namespace Content
{
class ReaderImageProvider final : public QQuickImageProvider
{
public:
    ReaderImageProvider()
        : QQuickImageProvider(QQuickImageProvider::Image, QQuickImageProvider::ForceAsynchronousImageLoading)
    {
    }

    void clear(const quint64 generation)
    {
        QWriteLocker guard(&m_lock);
        m_generation = generation;
        m_assets.clear();
    }

    void add(const quint64 generation, const QString &id, const QByteArray &bytes)
    {
        QWriteLocker guard(&m_lock);
        if (generation == m_generation && id.size() == 64 && bytes.size() <= 8 * 1024 * 1024)
            m_assets.insert(id, bytes);
    }

    QImage requestImage(const QString &id, QSize *size, const QSize &requestedSize) override
    {
        QByteArray data;
        const auto separator = id.indexOf(QLatin1Char('/'));
        if (separator < 0)
            return {};
        {
            QReadLocker guard(&m_lock);
            if (id.left(separator).toULongLong() != m_generation)
                return {};
            data = m_assets.value(id.mid(separator + 1));
        }
        if (data.isEmpty())
            return {};

        QBuffer buffer(&data);
        buffer.open(QIODevice::ReadOnly);
        QImageReader reader(&buffer);
        const QSize original = reader.size();
        if (!original.isValid())
            return {};
        if (size)
            *size = original;
        QSize bounds(4096, 4096);
        if (requestedSize.isValid())
            bounds = QSize(qMin(bounds.width(), requestedSize.width()),
                qMin(bounds.height(), requestedSize.height()));
        const QSize decoded = original.width() > bounds.width() || original.height() > bounds.height()
            ? original.scaled(bounds, Qt::KeepAspectRatio) : original;
        if (decoded != original)
            reader.setScaledSize(decoded);
        return reader.read();
    }

private:
    QReadWriteLock m_lock;
    quint64 m_generation = 0;
    QHash<QString, QByteArray> m_assets;
};
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
    connect(m_controller, &Qbutt::ContentController::downloadQueued, this, &ContentView::downloadQueued);
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
}

Content::ContentView::~ContentView()
{
    m_controller->closeBrowserRelay();
}

QQuickWidget *Content::ContentView::createDownloadsView(QWidget *parent)
{
    auto *view = new QQuickWidget(parent);
    view->setObjectName(QStringLiteral("downloadsView"));
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

void Content::ContentView::initDownloads(const QString &destination)
{
    m_controller->initDownloads(destination);
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
