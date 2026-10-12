// SPDX-License-Identifier: GPL-2.0-or-later
#include "quick-handoff.h"

#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QCheckBox>
#include <QDialogButtonBox>
#include <QDir>
#include <QDirIterator>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QKeyEvent>
#include <QLabel>
#include <QMouseEvent>
#include <QPointer>
#include <QPushButton>
#include <QQuickItem>
#include <QRegularExpression>
#include <QThread>
#include <QTimer>

#include "app/application.h"
#include "base/bittorrent/addtorrenterror.h"
#include "base/bittorrent/session.h"
#include "base/bittorrent/torrentdescriptor.h"
#include "base/net/portforwarder.h"
#include "base/preferences.h"
#include "base/profile.h"
#include "base/utils/fs.h"
#include "gui/addnewtorrentdialog.h"
#include "gui/fspathedit.h"
#include "gui/guiaddtorrentmanager.h"
#include "gui/mainwindow.h"
#include "gui/quick/contentview.h"
#include "qbutt_quick/src/controller.cxxqt.h"

using namespace Qt::Literals::StringLiterals;

namespace
{
    void require(const bool condition, const QString &message)
    {
        if (!condition)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const std::function<bool()> &ready, const int timeout, const QString &operation)
    {
        if (ready())
            return;

        QElapsedTimer elapsed;
        elapsed.start();
        QEventLoop loop;
        QTimer timer;
        timer.setInterval(20);
        QObject::connect(&timer, &QTimer::timeout, &loop, [&]
        {
            if (ready() || (elapsed.elapsed() >= timeout))
                loop.quit();
        });
        timer.start();
        loop.exec();
        require((elapsed.elapsed() < timeout) && ready(), operation + u" timed out"_s);
    }

    QString value(QAbstractItemModel *model, const int row, const QByteArray &name)
    {
        const auto roles = model->roleNames();
        for (auto it = roles.cbegin(); it != roles.cend(); ++it)
        {
            if (it.value() == name)
                return model->data(model->index(row, 0), it.key()).toString();
        }
        throw std::runtime_error("Missing model role: " + name.toStdString());
    }
}

void QuickHandoffAcceptance::run(Application &application, const QJsonObject &spec, QJsonObject &evidence)
{
    auto *session = BitTorrent::Session::instance();
    auto *manager = static_cast<IGUIApplication &>(application).addTorrentManager();
    require(session && manager, u"Application did not initialize its torrent services"_s);
    require(!Preferences::instance()->isSearchEnabled(), u"Legacy search must be disabled in this fixture"_s);
    require(session->torrents().isEmpty(), u"Fixture profile contains torrents"_s);
    require(session->networkInterfaceAddress() == u"127.0.0.1", u"Torrent traffic is not bound to loopback"_s);
    require(!session->isDHTEnabled() && !session->isLSDEnabled() && !session->isPeXEnabled(),
        u"Peer discovery is enabled in the fixture"_s);
    require(!Net::PortForwarder::instance()->isEnabled(), u"Port forwarding is enabled in the fixture"_s);

    const QString source = spec.value(u"source"_s).toString();
    require((source == u"aniliberty") || (source == u"rutor"), u"Unsupported handoff source"_s);
    const bool ruTor = source == u"rutor";
    const QString sourceLabel = ruTor ? u"RuTor"_s : u"AniLiberty"_s;
    const QString category = ruTor ? u"software"_s : u"anime"_s;

    const QDir payload(spec.value(u"payload"_s).toString());
    require(payload.exists() && payload.entryList(QDir::AllEntries | QDir::NoDotAndDotDot).isEmpty(),
        u"Fixture payload directory must exist and be empty"_s);
    const QDir screenshots(spec.value(u"screenshots"_s).toString());
    require(screenshots.exists(), u"Screenshot directory is missing"_s);

    application.mainWindow()->hide();
    Content::ContentView view(specialFolderLocation(SpecialFolder::Data).toString());
    view.setWindowTitle(u"qbutt"_s);
    view.resize(1280, 760);
    view.show();
    waitFor([&] { return view.status() != QQuickWidget::Loading; }, 10'000, u"QML load"_s);
    require(view.status() == QQuickWidget::Ready, u"Production ContentView did not load"_s);

    auto *controller = view.findChild<Qbutt::ContentController *>();
    require(controller && (controller->thread() == qApp->thread()), u"Missing GUI-thread content controller"_s);
    auto *works = qobject_cast<QAbstractItemModel *>(controller->property("works").value<QObject *>());
    auto *variants = qobject_cast<QAbstractItemModel *>(controller->property("variants").value<QObject *>());
    require(works && variants && (works != variants), u"Production content models are missing"_s);
    require((works->thread() == qApp->thread()) && (variants->thread() == qApp->thread()),
        u"Content models are not owned by the GUI thread"_s);
    auto *root = view.rootObject();
    require(root, u"Production QML root is missing"_s);
    auto *query = root->findChild<QQuickItem *>(u"contentQuery"_s);
    auto *categories = root->findChild<QQuickItem *>(u"contentCategory"_s);
    int categoryIndex = -1;
    require(categories && QMetaObject::invokeMethod(categories, "indexOfValue", Q_RETURN_ARG(int, categoryIndex),
        Q_ARG(QVariant, QVariant(category))) && (categoryIndex >= 0)
        && categories->setProperty("currentIndex", categoryIndex)
        && (categories->property("currentValue").toString() == category), u"QML category control is missing"_s);
    require(query && query->setProperty("text", ruTor ? u"Blender"_s : u"Frieren"_s), u"Search field is missing"_s);
    require(QMetaObject::invokeMethod(root, "submitSearch"), u"Search action is missing"_s);
    require(controller->property("busy").toBool(), u"Search did not start"_s);
    waitFor([&] { return !controller->property("busy").toBool(); }, 150'000, u"Catalog search"_s);
    require(controller->property("errorText").toString().isEmpty(), u"Catalog search failed"_s);

    QString workId;
    for (int row = 0; row < works->rowCount(); ++row)
    {
        const QString id = value(works, row, "workId");
        if ((value(works, row, "source") == sourceLabel) && (value(works, row, "category") == category)
            && (!ruTor || value(works, row, "title").contains(u"Blender"_s, Qt::CaseInsensitive))
            && (ruTor || (id == u"aniliberty:9542")))
        {
            workId = id;
            break;
        }
    }
    require(!workId.isEmpty() && (!ruTor || QRegularExpression(u"^rutor:[1-9][0-9]*$"_s).match(workId).hasMatch()),
        u"Search did not return the expected typed publication"_s);
    auto *results = root->findChild<QQuickItem *>(u"contentResults"_s);
    auto *filtered = results ? qobject_cast<QAbstractItemModel *>(results->property("model").value<QObject *>()) : nullptr;
    require(filtered && (filtered != works), u"Production QML category projection is missing"_s);
    int projectedRow = -1;
    for (int row = 0; row < filtered->rowCount(); ++row)
    {
        require(value(filtered, row, "category") == category, u"QML category filter included another category"_s);
        if (value(filtered, row, "workId") == workId)
            projectedRow = row;
    }
    require((projectedRow >= 0) && results->setProperty("currentIndex", projectedRow),
        u"Publication is absent from its QML category"_s);
    results->forceActiveFocus();
    QKeyEvent enter(QEvent::KeyPress, Qt::Key_Return, Qt::NoModifier);
    QCoreApplication::sendEvent(&view, &enter);
    QKeyEvent enterRelease(QEvent::KeyRelease, Qt::Key_Return, Qt::NoModifier);
    QCoreApplication::sendEvent(&view, &enterRelease);
    require(controller->property("selectedWorkId").toString() == workId, u"QML selected an unfiltered source row"_s);
    require(controller->property("inspecting").toBool(), u"Inspection did not start"_s);
    waitFor([&] { return !controller->property("inspecting").toBool(); }, 50'000, u"Publication inspection"_s);
    require(controller->property("errorText").toString().isEmpty(), u"Publication inspection failed"_s);
    require(controller->property("selectedWorkCategory").toString() == category, u"Fresh category differs from the source route"_s);
    require(!ruTor || controller->property("selectedWorkTitle").toString().contains(u"Blender"_s, Qt::CaseInsensitive),
        u"RuTor inspection returned an unrelated software title"_s);
    require(ruTor ? variants->rowCount() == 1 : variants->rowCount() >= 2,
        u"Source did not return the required exact torrent variants"_s);

    const int selectedRow = variants->rowCount() - 1;
    const QString variantId = value(variants, selectedRow, "variantId");
    const QString infoHash = value(variants, selectedRow, "infoHash").toLower();
    require(value(variants, selectedRow, "kind") == u"torrent", u"Selected variant is not a torrent"_s);
    require(value(variants, selectedRow, "source") == sourceLabel, u"Selected variant belongs to another source"_s);
    require(QRegularExpression(u"^[0-9a-f]{40}$"_s).match(infoHash).hasMatch(),
        u"Selected torrent has no verifiable SHA-1 metadata"_s);
    const QString defaultInfoHash = value(variants, 0, "infoHash").toLower();
    if (ruTor)
        require(variantId == workId + u":torrent:"_s + infoHash, u"RuTor lost its full-hash exact selector"_s);
    else
        require((selectedRow > 0) && (infoHash != defaultInfoHash), u"Variants do not have distinct hashes"_s);
    auto *list = root->findChild<QQuickItem *>(u"contentVariants"_s);
    auto *download = root->findChild<QQuickItem *>(u"contentDownload"_s);
    require(list && download, u"Variant controls are missing"_s);
    list->forceActiveFocus();
    QKeyEvent lastVariant(QEvent::KeyPress, Qt::Key_End, Qt::NoModifier);
    QCoreApplication::sendEvent(&view, &lastVariant);
    QKeyEvent lastVariantRelease(QEvent::KeyRelease, Qt::Key_End, Qt::NoModifier);
    QCoreApplication::sendEvent(&view, &lastVariantRelease);
    waitFor([&]
    {
        auto *item = list->property("currentItem").value<QObject *>();
        return item && (item->property("variantId").toString() == variantId)
            && (controller->property("selectedVariantId").toString() == variantId) && download->isEnabled();
    }, 5000, u"Exact QML variant selection"_s);
    require(view.grabFramebuffer().save(screenshots.filePath(u"quick-search.png"_s)), u"Could not capture the selected variant"_s);

    int handoffs = 0;
    int added = 0;
    int failures = 0;
    int cancellations = 0;
    bool guiThread = true;
    bool managerAccepted = false;
    BitTorrent::InfoHash cancelledHash;
    QObject observation;
    QObject::connect(manager, &AddTorrentManager::torrentAdded, &observation, [&] { ++added; });
    QObject::connect(manager, &AddTorrentManager::addTorrentFailed, &observation, [&] { ++failures; });
    QObject::connect(manager, &GUIAddTorrentManager::addTorrentCancelled, &observation,
        [&](const BitTorrent::InfoHash &hash)
    {
        ++cancellations;
        cancelledHash = hash;
    });
    QObject::connect(&view, &Content::ContentView::torrentReady, &observation, [&](const QString &magnet)
    {
        ++handoffs;
        guiThread = guiThread && (QThread::currentThread() == qApp->thread());
        BitTorrent::AddTorrentParams params;
        params.savePath = Path(payload.absolutePath());
        params.useAutoTMM = false;
        params.useDownloadPath = false;
        params.addStopped = true;
        managerAccepted = manager->addTorrent(magnet, params, AddTorrentOption::ShowDialog);
    });

    const QPointF point = download->mapToScene(QPointF(download->width() / 2, download->height() / 2));
    const QPointF global = view.mapToGlobal(point.toPoint());
    QMouseEvent press(QEvent::MouseButtonPress, point, global, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
    QCoreApplication::sendEvent(&view, &press);
    QMouseEvent release(QEvent::MouseButtonRelease, point, global, Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
    QCoreApplication::sendEvent(&view, &release);
    require(controller->property("resolving").toBool(), u"Download button did not resolve the selected variant"_s);
    waitFor([&] { return !controller->property("resolving").toBool(); }, 50'000, u"Exact torrent resolution"_s);
    require(controller->property("errorText").toString().isEmpty(), u"Exact resolution failed"_s);
    require(guiThread && (handoffs == 1) && managerAccepted && (failures == 0), u"Native torrent handoff failed"_s);

    QList<AddNewTorrentDialog *> dialogs;
    for (QWidget *widget : QApplication::topLevelWidgets())
    {
        if (auto *dialog = qobject_cast<AddNewTorrentDialog *>(widget); dialog && dialog->isVisible())
            dialogs.append(dialog);
    }
    require(dialogs.size() == 1, u"Handoff did not open exactly one native torrent dialog"_s);
    QPointer<AddNewTorrentDialog> dialog = dialogs.first();
    auto *hashLabel = dialog->findChild<QLabel *>(u"labelInfohash1Data"_s);
    auto *savePath = dialog->findChild<FileSystemPathEdit *>(u"savePath"_s);
    auto *start = dialog->findChild<QCheckBox *>(u"startTorrentCheckBox"_s);
    auto *buttons = dialog->findChild<QDialogButtonBox *>(u"buttonBox"_s);
    require(hashLabel && (hashLabel->text().compare(infoHash, Qt::CaseInsensitive) == 0),
        u"Native dialog contains a different torrent hash"_s);
    require(savePath && (savePath->selectedPath() == Path(payload.absolutePath())), u"Native dialog escaped the fixture destination"_s);
    require(start && !start->isChecked(), u"Native dialog is not configured to add a stopped torrent"_s);
    require(buttons && buttons->button(QDialogButtonBox::Cancel), u"Native Cancel button is missing"_s);
    require(dialog->grab().save(screenshots.filePath(u"quick-add-torrent.png"_s)), u"Could not capture the native torrent dialog"_s);

    BitTorrent::InfoHash rejectedHash;
    QObject::connect(dialog, &AddNewTorrentDialog::torrentRejected, &observation,
        [&](const BitTorrent::TorrentDescriptor &descriptor) { rejectedHash = descriptor.infoHash(); });
    QTimer::singleShot(0, buttons->button(QDialogButtonBox::Cancel), &QPushButton::click);
    waitFor([&] { return dialog.isNull(); }, 5000, u"Cancelled torrent dialog destruction"_s);
    require((cancellations == 1) && (cancelledHash == rejectedHash)
        && (rejectedHash.v1().toString().compare(infoHash, Qt::CaseInsensitive) == 0),
        u"Native cancellation did not preserve the exact selected torrent"_s);
    require(!session->isKnownTorrent(rejectedHash) && session->torrents().isEmpty() && (added == 0) && (failures == 0),
        u"Cancelled handoff left a torrent or metadata request registered"_s);
    require(payload.entryList(QDir::AllEntries | QDir::NoDotAndDotDot).isEmpty(), u"Cancelled handoff wrote payload files"_s);
    // Inspect before shutdown removes qbutt's temporary metadata directory.
    const Path metadataPath = Utils::Fs::tempPath() / Path(rejectedHash.toTorrentID().toString());
    QDirIterator metadataFiles(metadataPath.toString(), QDir::Files | QDir::Hidden | QDir::System, QDirIterator::Subdirectories);
    require(!metadataFiles.hasNext(), u"Metadata request wrote a payload file"_s);

    QJsonArray checks = evidence.value(u"checks"_s).toArray();
    checks.append(QJsonObject {
        {u"name"_s, u"quick-handoff"_s},
        {u"source"_s, source},
        {u"category"_s, category},
        {u"workId"_s, workId},
        {u"variantId"_s, variantId},
        {u"infoHash"_s, infoHash},
        {u"selectedRow"_s, selectedRow},
        {u"projectedRow"_s, projectedRow},
        {u"variantCount"_s, variants->rowCount()},
        {u"defaultInfoHash"_s, defaultInfoHash},
        {u"handoffs"_s, handoffs},
        {u"cancellations"_s, cancellations},
        {u"exactDialogHash"_s, true},
        {u"torrentCount"_s, session->torrents().size()},
        {u"payloadEmpty"_s, true},
        {u"metadataPayloadEmpty"_s, true}
    });
    evidence.insert(u"checks"_s, checks);
}
