// SPDX-License-Identifier: GPL-2.0-or-later
#include "quick-handoff.h"

#include <functional>
#include <memory>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QAction>
#include <QApplication>
#include <QCheckBox>
#include <QCryptographicHash>
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
#include <QPalette>
#include <QPointer>
#include <QPushButton>
#include <QQmlError>
#include <QQuickItem>
#include <QRegularExpression>
#include <QStringList>
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
#include "gui/uithememanager.h"
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

    void clickItem(QQuickWidget &view, QQuickItem *item)
    {
        const QPointF point = item->mapToScene(QPointF(item->width() / 2, item->height() / 2));
        const QPointF global = view.mapToGlobal(point.toPoint());
        QMouseEvent press(QEvent::MouseButtonPress, point, global, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(&view, &press);
        QMouseEvent release(QEvent::MouseButtonRelease, point, global, Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
        QCoreApplication::sendEvent(&view, &release);
    }
}

void QuickHandoffAcceptance::run(Application &application, const QJsonObject &spec, QJsonObject &evidence)
{
    auto *session = BitTorrent::Session::instance();
    auto *manager = static_cast<IGUIApplication &>(application).addTorrentManager();
    const bool mainContent = spec.value(u"mode"_s).toString() == u"main-content";
    require(session && manager, u"Application did not initialize its torrent services"_s);
    require(Preferences::instance()->isSearchEnabled() == mainContent, u"Unexpected search tab configuration"_s);
    require(session->torrents().isEmpty(), u"Fixture profile contains torrents"_s);
    require(session->networkInterfaceAddress() == u"127.0.0.1", u"Torrent traffic is not bound to loopback"_s);
    require(!session->isDHTEnabled() && !session->isLSDEnabled() && !session->isPeXEnabled(),
        u"Peer discovery is enabled in the fixture"_s);
    require(!Net::PortForwarder::instance()->isEnabled(), u"Port forwarding is enabled in the fixture"_s);

    const QString source = spec.value(u"source"_s).toString();
    require((source == u"aniliberty") || (source == u"rutor") || (source == u"limetorrents")
            || (source == u"nnmclub") || (source == u"dodi") || (source == u"byrut")
            || (source == u"freetp") || (source == u"fitgirl") || (source == u"rutracker")
            || (source == u"onlinefix") || (source == u"igruha"),
        u"Unsupported handoff source"_s);
    const bool ruTor = source == u"rutor";
    const bool limeTorrents = source == u"limetorrents";
    const bool nnmClub = source == u"nnmclub";
    const bool ruTracker = source == u"rutracker";
    const bool dodi = source == u"dodi";
    const bool byRut = source == u"byrut";
    const bool freeTP = source == u"freetp";
    const bool fitGirl = source == u"fitgirl";
    const bool onlineFix = source == u"onlinefix";
    const bool igruha = source == u"igruha";
    const bool torrentFile = dodi || byRut || freeTP || onlineFix || igruha;
    const bool singleTorrent = ruTor || limeTorrents || nnmClub || ruTracker || torrentFile;
    QString sourceLabel = u"AniLiberty"_s;
    QString category = u"anime"_s;
    QString queryText = u"Frieren"_s;
    if (ruTor)
    {
        sourceLabel = u"RuTor"_s;
        category = u"software"_s;
        queryText = u"Blender"_s;
    }
    else if (limeTorrents)
    {
        sourceLabel = u"LimeTorrents"_s;
        category = u"movies"_s;
        queryText = u"Big Buck Bunny"_s;
    }
    else if (nnmClub)
    {
        sourceLabel = u"NNM-Club"_s;
        category = u"movies"_s;
        queryText = u"Sintel"_s;
    }
    else if (ruTracker)
    {
        sourceLabel = u"RuTracker"_s;
        category = u"movies"_s;
        queryText = u"Sintel"_s;
    }
    else if (dodi)
    {
        sourceLabel = u"DODI Repacks"_s;
        category = u"games"_s;
        queryText = u"Devil May Cry"_s;
    }
    else if (byRut)
    {
        sourceLabel = u"ByRut"_s;
        category = u"games"_s;
        queryText = u"Stardew Valley"_s;
    }
    else if (freeTP)
    {
        sourceLabel = u"FreeTP"_s;
        category = u"games"_s;
        queryText = u"КЛЕТЬ"_s;
    }
    else if (fitGirl)
    {
        sourceLabel = u"FitGirl"_s;
        category = u"games"_s;
        queryText = u"Cyberpunk"_s;
    }
    else if (onlineFix)
    {
        sourceLabel = u"Online-Fix"_s;
        category = u"games"_s;
        queryText = u"Stardew Valley"_s;
    }
    else if (igruha)
    {
        sourceLabel = u"Torrent-Igruha"_s;
        category = u"games"_s;
        queryText = u"OpenTTD"_s;
    }

    const QDir payload(spec.value(u"payload"_s).toString());
    require(payload.exists() && payload.entryList(QDir::AllEntries | QDir::NoDotAndDotDot).isEmpty(),
        u"Fixture payload directory must exist and be empty"_s);
    const QDir screenshots(spec.value(u"screenshots"_s).toString());
    require(screenshots.exists(), u"Screenshot directory is missing"_s);

    std::unique_ptr<Content::ContentView> isolatedView;
    Content::ContentView *contentView = nullptr;
    if (mainContent)
    {
        auto *window = application.mainWindow();
        const auto views = window->findChildren<Content::ContentView *>();
        require((views.size() == 1) && (window->currentTabWidget() == views.first()),
            u"MainWindow did not open its only native content view"_s);
        contentView = views.first();
        window->resize(1440, 900);
        window->show();
    }
    else
    {
        application.mainWindow()->hide();
        isolatedView = std::make_unique<Content::ContentView>(specialFolderLocation(SpecialFolder::Data).toString());
        contentView = isolatedView.get();
        contentView->setWindowTitle(u"qbutt"_s);
        contentView->resize(1280, 760);
        contentView->show();
    }
    Content::ContentView &view = *contentView;
    waitFor([&] { return view.status() != QQuickWidget::Loading; }, 10'000, u"QML load"_s);
    QStringList qmlErrors;
    for (const QQmlError &error : view.errors())
        qmlErrors.append(error.toString());
    require(view.status() == QQuickWidget::Ready,
        u"Production ContentView did not load: "_s + qmlErrors.join(u"; "_s));

    auto *controller = view.findChild<Qbutt::ContentController *>();
    require(controller && (controller->thread() == qApp->thread()), u"Missing GUI-thread content controller"_s);
    auto *works = qobject_cast<QAbstractItemModel *>(controller->property("works").value<QObject *>());
    auto *variants = qobject_cast<QAbstractItemModel *>(controller->property("variants").value<QObject *>());
    require(works && variants && (works != variants), u"Production content models are missing"_s);
    require((works->thread() == qApp->thread()) && (variants->thread() == qApp->thread()),
        u"Content models are not owned by the GUI thread"_s);
    auto *root = view.rootObject();
    require(root, u"Production QML root is missing"_s);
    waitFor([&] { return controller->getLibraryState() == Qbutt::Library::LibraryState::Ready; },
        10'000, u"Profile content library"_s);
    waitFor([&]
    {
        return controller->property("sourceSettingsState").value<Qbutt::Sources::SettingsState>()
            == Qbutt::Sources::SettingsState::Ready;
    }, 10'000, u"Source settings"_s);
    auto *sources = qobject_cast<QAbstractItemModel *>(controller->property("sources").value<QObject *>());
    require(sources && (sources->rowCount() > 0), u"Source model is missing"_s);
    if (ruTracker)
    {
        view.setBrowserChoice(u"edge"_s);
        controller->watchBrowser();
        waitFor([&]
        {
            return controller->property("browserConnectionState").value<Qbutt::Browser::ConnectionState>()
                == Qbutt::Browser::ConnectionState::Connected;
        }, 65'000, u"Existing authenticated Edge connection"_s);
        controller->closeBrowserSettings();
    }
    for (int row = 0; row < sources->rowCount(); ++row)
    {
        const QString id = value(sources, row, "sourceId");
        require(controller->setSourceEnabled(id, id == source), u"Source selection was rejected"_s);
    }
    waitFor([&]
    {
        for (int row = 0; row < sources->rowCount(); ++row)
        {
            const bool selected = value(sources, row, "sourceId") == source;
            if ((value(sources, row, "pending") == u"true")
                || (value(sources, row, "enabled") != (selected ? u"true" : u"false")))
            {
                return false;
            }
        }
        return true;
    }, 10'000, u"Isolated provider selection"_s);
    if (igruha)
    {
        controller->watchBrowser();
        waitFor([&]
        {
            return controller->property("browserConnectionState").value<Qbutt::Browser::ConnectionState>()
                == Qbutt::Browser::ConnectionState::Connected;
        }, 30'000, u"Connected Igruha browser session"_s);
        controller->closeBrowserSettings();
    }
    if (mainContent)
    {
        auto *background = root->property("background").value<QQuickItem *>();
        auto *applicationPalette = root->findChild<QObject *>(u"contentApplicationPalette"_s);
        auto *search = root->findChild<QQuickItem *>(u"contentSearch"_s);
        auto *buttonTheme = search ? search->property("theme").value<QObject *>() : nullptr;
        auto *buttonLabel = search ? search->property("contentItem").value<QQuickItem *>() : nullptr;
        require(background && applicationPalette && buttonTheme && buttonLabel, u"Native QML theme objects are missing"_s);
        require((applicationPalette->property("window").value<QColor>() == qApp->palette().color(QPalette::Window))
            && (applicationPalette->property("windowText").value<QColor>() == qApp->palette().color(QPalette::WindowText)),
            u"QML SystemPalette differs from QApplication"_s);
        require(background->property("color").value<QColor>() == view.palette().color(QPalette::Window),
            u"Content palette differs: QML="_s + background->property("color").value<QColor>().name()
            + u", QWidget="_s + view.palette().color(QPalette::Window).name());
        view.giveFocusToSearchInput();
        auto *input = root->findChild<QQuickItem *>(u"contentQuery"_s);
        require(input && input->hasActiveFocus(), u"MainWindow focus action did not focus QML search"_s);
        input->setProperty("text", queryText);
        UIThemeManager::instance()->previewColorScheme(ColorScheme::Dark);
        waitFor([&]
        {
            return (background->property("color").value<QColor>() == QColor(u"#202123"_s))
                && (applicationPalette->property("window").value<QColor>() == qApp->palette().color(QPalette::Window))
                && (applicationPalette->property("windowText").value<QColor>() == qApp->palette().color(QPalette::WindowText))
                && (buttonTheme->property("background").value<QColor>() == QColor(u"#202123"_s))
                && (buttonTheme->property("foreground").value<QColor>() == qApp->palette().color(QPalette::WindowText))
                && (buttonLabel->property("color").value<QColor>() == QColor(Qt::white));
        },
            5000, u"Main content dark palette"_s);
        UIThemeManager::instance()->previewColorScheme(ColorScheme::Light);
        waitFor([&]
        {
            return (background->property("color").value<QColor>() == QColor(u"#f5f6f7"_s))
                && (applicationPalette->property("window").value<QColor>() == qApp->palette().color(QPalette::Window))
                && (applicationPalette->property("windowText").value<QColor>() == qApp->palette().color(QPalette::WindowText))
                && (buttonTheme->property("background").value<QColor>() == QColor(u"#f5f6f7"_s))
                && (buttonTheme->property("foreground").value<QColor>() == qApp->palette().color(QPalette::WindowText))
                && (buttonLabel->property("color").value<QColor>() == QColor(Qt::white));
        },
            5000, u"Main content light palette"_s);
        auto *window = application.mainWindow();
        auto *downloads = window->findChild<QAction *>(u"actionContentDownloads"_s);
        require(downloads && downloads->isEnabled() && (downloads->shortcut() == QKeySequence(Qt::CTRL | Qt::Key_J))
            && window->findChildren<QQuickWidget *>(u"downloadsView"_s).isEmpty(),
            u"Direct downloads have no lazy manual entry"_s);
        downloads->trigger();
        auto *jobs = window->findChild<QQuickWidget *>(u"downloadsView"_s);
        require(jobs && (window->currentTabWidget() == jobs), u"Manual Downloads action did not open the native view"_s);
        waitFor([&] { return jobs->status() != QQuickWidget::Loading; }, 10'000, u"Downloads QML load"_s);
        require(jobs->status() == QQuickWidget::Ready && jobs->rootObject(),
            u"Downloads QML did not load"_s);
        waitFor([&] { return controller->property("downloadsState").toString() != u"loading"_s; },
            10'000, u"Native download queue"_s);
        require(controller->property("downloadsState").toString() == u"ready"_s,
            u"Native download queue did not open"_s);
        auto *downloadList = jobs->rootObject()->findChild<QQuickItem *>(u"contentDownloads"_s);
        require(jobs->rootObject()->property("backend").value<QObject *>() == controller
            && downloadList
            && downloadList->property("model").value<QObject *>()
                == controller->property("downloads").value<QObject *>(),
            u"Downloads view does not share the content controller and queue model"_s);
        downloads->trigger();
        require(window->findChildren<QQuickWidget *>(u"downloadsView"_s).size() == 1
            && window->currentTabWidget() == jobs,
            u"Downloads action duplicated its queue view"_s);
        require(QMetaObject::invokeMethod(window, "displaySearchTab") && (window->currentTabWidget() == &view),
            u"Returning from Downloads lost the native search view"_s);
    }
    auto *query = root->findChild<QQuickItem *>(u"contentQuery"_s);
    auto *categories = root->findChild<QQuickItem *>(u"contentCategory"_s);
    int categoryIndex = -1;
    require(categories && QMetaObject::invokeMethod(categories, "indexOfValue", Q_RETURN_ARG(int, categoryIndex),
        Q_ARG(QVariant, QVariant(category))) && (categoryIndex >= 0)
        && categories->setProperty("currentIndex", categoryIndex)
        && (categories->property("currentValue").toString() == category), u"QML category control is missing"_s);
    require(query && query->setProperty("text", queryText), u"Search field is missing"_s);
    require(QMetaObject::invokeMethod(root, "submitSearch"), u"Search action is missing"_s);
    require(controller->property("busy").toBool(), u"Search did not start"_s);
    waitFor([&] { return !controller->property("busy").toBool(); }, 150'000, u"Catalog search"_s);
    require(controller->property("errorText").toString().isEmpty(), u"Catalog search failed"_s);
    for (int row = 0; row < sources->rowCount(); ++row)
    {
        if (value(sources, row, "sourceId") == source)
        {
            require(value(sources, row, "failed") != u"true",
                u"Catalog source failed: "_s + value(sources, row, "failureText"));
        }
    }

    QString expectedWorkId = u"aniliberty:9542"_s;
    if (limeTorrents)
        expectedWorkId = u"limetorrents:Big-Buck-Bunny-4K-UHD-HFR-60fps-FLAC-WEBDL-2160p-X264-mkv-torrent-9616433.html"_s;
    else if (nnmClub)
        expectedWorkId = u"nnmclub:906013"_s;
    else if (ruTracker)
        expectedWorkId = u"rutracker:5026149"_s;
    else if (dodi)
        expectedWorkId = u"dodi:2643--devil-may-cry-5"_s;
    else if (byRut)
        expectedWorkId = u"byrut:2799-stardew-valley"_s;
    else if (freeTP)
        expectedWorkId = u"freetp:6327"_s;
    else if (fitGirl)
        expectedWorkId = u"fitgirl:cyberpunk-2077"_s;
    else if (onlineFix)
        expectedWorkId = u"onlinefix:games/sandbox/16274-stardew-valley-po-seti.html"_s;
    else if (igruha)
        expectedWorkId = u"igruha:7861-open-transport-tycoon-deluxe"_s;
    QString workId;
    for (int row = 0; row < works->rowCount(); ++row)
    {
        const QString id = value(works, row, "workId");
        if ((value(works, row, "source") == sourceLabel) && (value(works, row, "category") == category)
            && (!ruTor || value(works, row, "title").contains(u"Blender"_s, Qt::CaseInsensitive))
            && (ruTor || (id == expectedWorkId)))
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
    require(singleTorrent ? variants->rowCount() == 1 : variants->rowCount() >= 2,
        u"Source did not return the required exact torrent variants"_s);

    QList<int> torrentRows;
    for (int row = 0; row < variants->rowCount(); ++row)
    {
        if (value(variants, row, "kind") == u"torrent")
            torrentRows.append(row);
    }
    require(singleTorrent ? torrentRows.size() == 1 : torrentRows.size() >= 2, u"Exact torrent variants are missing"_s);
    const int selectedRow = torrentRows.constLast();
    const QString variantId = value(variants, selectedRow, "variantId");
    const QString infoHash = dodi ? u"b34fa7dfc9c95baf8509f6bd90a4ce5e1fde22e1"_s
        : value(variants, selectedRow, "infoHash").toLower();
    require(value(variants, selectedRow, "kind") == u"torrent", u"Selected variant is not a torrent"_s);
    require(value(variants, selectedRow, "source") == sourceLabel, u"Selected variant belongs to another source"_s);
    require(QRegularExpression(u"^[0-9a-f]{40}$"_s).match(infoHash).hasMatch(),
        u"Selected torrent has no verifiable SHA-1 metadata"_s);
    const QString defaultInfoHash = value(variants, torrentRows.constFirst(), "infoHash").toLower();
    if (dodi)
        require(value(variants, selectedRow, "infoHash").isEmpty()
            && (variantId == workId + u":torrent:vpe2b0x0a9zw"_s),
            u"DODI fabricated a hash or changed its exact FileMe selection"_s);
    else if (singleTorrent)
        require(variantId == workId + u":torrent:"_s + infoHash, u"Torrent source lost its full-hash exact selector"_s);
    else
        require((selectedRow > 0) && (infoHash != defaultInfoHash), u"Variants do not have distinct hashes"_s);
    if (limeTorrents)
        require(infoHash == u"c97f976894b81bb0f0befea584c9a81ad3bd9f5a", u"Known public-domain torrent hash changed"_s);
    if (nnmClub)
        require(infoHash == u"eeddbd381bc734373bccf170a17d3bba23fa33a3", u"Known Sintel torrent hash changed"_s);
    if (ruTracker)
        require(infoHash == u"cfcdab5d3c5efb529590160ea84fc419977fbd67", u"Known RuTracker Sintel torrent hash changed"_s);
    if (byRut)
        require(infoHash == u"942e9eeace33f7cfefa4fbcc129a1a9f12097638", u"Known Stardew Valley torrent hash changed"_s);
    if (freeTP)
        require(infoHash == u"e54e92ed62a49401fc224642e65f8ca03709c95e", u"Known KLETKA torrent hash changed"_s);
    if (fitGirl)
        require(infoHash == u"f081eec38f08f5d663db807b39c5c33bb1049e2f", u"Known Cyberpunk variant hash changed"_s);
    if (onlineFix)
        require(infoHash == u"063043892eac8f94c5eb249669548a3f93b7c3a5"
            && value(variants, selectedRow, "label").startsWith(u"Full game:"_s),
            u"Online-Fix full-game torrent changed"_s);
    if (igruha)
        require(infoHash == u"1ea103b38144f9bd91cb8e979949de4341010484", u"Known OpenTTD torrent hash changed"_s);
    auto *list = root->findChild<QQuickItem *>(u"contentVariants"_s);
    auto *download = root->findChild<QQuickItem *>(u"contentDownload"_s);
    require(list && download, u"Variant controls are missing"_s);
    auto *visibleVariants = qobject_cast<QAbstractItemModel *>(list->property("model").value<QObject *>());
    require(visibleVariants, u"Visible variant model is missing"_s);
    int visibleRow = -1;
    for (int row = 0; row < visibleVariants->rowCount(); ++row)
    {
        if (value(visibleVariants, row, "variantId") == variantId)
            visibleRow = row;
    }
    require(visibleRow >= 0, u"Exact torrent is absent from the visible variants"_s);
    list->forceActiveFocus();
    for (int step = 0; step <= visibleRow; ++step)
    {
        const Qt::Key key = step == 0 ? Qt::Key_Home : Qt::Key_Down;
        QKeyEvent press(QEvent::KeyPress, key, Qt::NoModifier);
        QCoreApplication::sendEvent(&view, &press);
        QKeyEvent release(QEvent::KeyRelease, key, Qt::NoModifier);
        QCoreApplication::sendEvent(&view, &release);
    }
    waitFor([&]
    {
        auto *item = list->property("currentItem").value<QObject *>();
        return item && (item->property("variantId").toString() == variantId)
            && (controller->property("selectedVariantId").toString() == variantId) && download->isEnabled();
    }, 5000, u"Exact QML variant selection"_s);
    require(view.grabFramebuffer().save(screenshots.filePath(u"quick-search.png"_s)), u"Could not capture the selected variant"_s);

    int handoffs = 0;
    int metadataBytes = 0;
    QString metadataSha256;
    int added = 0;
    int failures = 0;
    int cancellations = 0;
    bool guiThread = true;
    bool managerAccepted = false;
    BitTorrent::InfoHash cancelledHash;
    QObject observation;
    BitTorrent::AddTorrentParams params;
    params.savePath = Path(payload.absolutePath());
    params.useAutoTMM = false;
    params.useDownloadPath = false;
    params.addStopped = true;

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
        if (mainContent)
        {
            managerAccepted = true;
            return;
        }
        managerAccepted = manager->addTorrent(magnet, params, AddTorrentOption::ShowDialog);
    });
    QObject::connect(&view, &Content::ContentView::torrentMetadataReady, &observation,
        [&](const QString &identity, const QByteArray &data)
    {
        require(torrentFile && (identity == variantId), u"Torrent bytes lost the exact selected publication identity"_s);
        const auto descriptor = BitTorrent::TorrentDescriptor::load(data);
        require(descriptor && (descriptor->infoHash().v1().toString() == infoHash),
            u"Source acquired different or malformed torrent bytes"_s);
        ++handoffs;
        metadataBytes = data.size();
        metadataSha256 = QString::fromLatin1(QCryptographicHash::hash(data, QCryptographicHash::Sha256).toHex());
        guiThread = guiThread && (QThread::currentThread() == qApp->thread());
        managerAccepted = mainContent || manager->addTorrentData(identity, data);
    });

    clickItem(view, download);
    require(controller->property("resolving").toBool(), u"Download button did not resolve the selected variant"_s);
    waitFor([&] { return !controller->property("resolving").toBool(); }, 50'000, u"Exact torrent resolution"_s);
    require(controller->property("errorText").toString().isEmpty(), u"Exact resolution failed"_s);
    require(guiThread && (handoffs == 1) && managerAccepted && (failures == 0), u"Native torrent handoff failed"_s);
    if (torrentFile)
        require((metadataBytes > 0) && (metadataBytes <= 10 * 1024 * 1024), u"Torrent metadata bytes were not bounded"_s);

    QList<AddNewTorrentDialog *> dialogs;
    waitFor([&]
    {
        dialogs.clear();
        for (QWidget *widget : QApplication::topLevelWidgets())
        {
            if (auto *dialog = qobject_cast<AddNewTorrentDialog *>(widget); dialog && dialog->isVisible())
                dialogs.append(dialog);
        }
        return !dialogs.isEmpty();
    }, 5000, u"Native torrent dialog"_s);
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
    QJsonObject check {
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
        {u"metadataBytes"_s, metadataBytes},
        {u"metadataSha256"_s, metadataSha256},
        {u"cancellations"_s, cancellations},
        {u"exactDialogHash"_s, true},
        {u"torrentCount"_s, session->torrents().size()},
        {u"payloadEmpty"_s, true},
        {u"metadataPayloadEmpty"_s, true},
        {u"mainContent"_s, mainContent}
    };
    if (mainContent)
    {
        require(application.mainWindow()->grab().save(screenshots.filePath(u"main-content.png"_s)),
            u"Could not capture main content view"_s);
        require(!controller->getSelectedFavorite(), u"Fresh profile unexpectedly has a favorite"_s);
        auto *favorite = root->findChild<QQuickItem *>(u"contentFavorite"_s);
        require(favorite && favorite->isEnabled(), u"Main favorite action is unavailable"_s);
        clickItem(view, favorite);
        require(controller->property("libraryPending").toBool(), u"Favorite did not enqueue an accepted write"_s);
        auto *searchAction = application.mainWindow()->findChild<QAction *>(u"actionSearchWidget"_s);
        require(searchAction && searchAction->isChecked(), u"Search visibility action is missing"_s);
        searchAction->trigger();
        require(!searchAction->isChecked() && !view.isVisible()
            && (application.mainWindow()->findChild<Content::ContentView *>() == &view)
            && (view.findChild<Qbutt::ContentController *>() == controller),
            u"Hiding Search destroyed its accepted library write owner"_s);
        searchAction->trigger();
        require(searchAction->isChecked() && (application.mainWindow()->findChildren<Content::ContentView *>().size() == 1)
            && (view.findChild<Qbutt::ContentController *>() == controller),
            u"Reopening Search created a second library owner"_s);
        require(QMetaObject::invokeMethod(application.mainWindow(), "displaySearchTab"), u"Could not return to Search"_s);
        waitFor([&] { return controller->getSelectedFavorite() && !controller->property("libraryPending").toBool(); },
            5000, u"Confirmed favorite after hiding and reopening Search"_s);
        require(controller->getSelectedWorkId() == workId && controller->getSelectedVariantId() == variantId,
            u"Search visibility changed the exact selection"_s);
        check[u"oneLibraryOwnerAcrossVisibility"_s] = true;
        clickItem(view, favorite);
        waitFor([&] { return !controller->getSelectedFavorite() && !controller->property("libraryPending").toBool(); },
            5000, u"Cleared favorite before Exit fixture"_s);
        clickItem(view, favorite);
        require(controller->property("libraryPending").toBool(), u"Exit did not retain an accepted favorite write"_s);
        check[u"favoriteWritePendingAtExit"_s] = true;
        check[u"libraryPath"_s] = specialFolderLocation(SpecialFolder::Data).toString();
    }
    checks.append(check);
    evidence.insert(u"checks"_s, checks);
}
