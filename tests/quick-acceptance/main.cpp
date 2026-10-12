// SPDX-License-Identifier: GPL-2.0-or-later
#include <atomic>
#include <functional>
#include <memory>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QCommandLineParser>
#include <QDebug>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QKeyEvent>
#include <QList>
#include <QLocale>
#include <QMainWindow>
#include <QPointer>
#include <QQuickItem>
#include <QSet>
#include <QStandardPaths>
#include <QStatusBar>
#include <QStyleHints>
#include <QTemporaryDir>
#include <QThread>
#include <QTimer>
#include <QTranslator>
#include <QUrl>
#include <QUrlQuery>
#include <QVariant>

#include "history.h"
#include "library.h"
#include "reader-progress.h"
#include "reader-settings.h"
#include "reader.h"
#include "worklist.h"

#include "gui/quick/contentview.h"
#include "qbutt_quick/src/controller.cxxqt.h"

using namespace Qt::Literals::StringLiterals;

namespace
{
    void require(const bool accepted, const QString &message)
    {
        if (!accepted)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const std::function<bool()> &condition, const int timeout, const QString &operation)
    {
        if (condition())
            return;

        QElapsedTimer elapsed;
        elapsed.start();
        QEventLoop loop;
        QTimer timer;
        timer.setInterval(20);
        QObject::connect(&timer, &QTimer::timeout, &loop, [&]
        {
            if ((elapsed.elapsed() >= timeout) || condition())
                loop.quit();
        });
        timer.start();
        loop.exec();
        require((elapsed.elapsed() < timeout) && condition(), operation + u" timed out"_s);
    }

    int role(QAbstractItemModel *model, const QByteArray &name)
    {
        const QHash<int, QByteArray> roles = model->roleNames();
        for (auto iterator = roles.cbegin(); iterator != roles.cend(); ++iterator)
        {
            if (iterator.value() == name)
                return iterator.key();
        }
        throw std::runtime_error("Missing model role: " + name.toStdString());
    }

    QString value(QAbstractItemModel *model, const int row, const QByteArray &name)
    {
        return model->data(model->index(row, 0), role(model, name)).toString();
    }

    QAbstractItemModel *modelProperty(QObject *controller, const char *name)
    {
        auto *model = qobject_cast<QAbstractItemModel *>(controller->property(name).value<QObject *>());
        require(model, u"Controller property is not a QAbstractItemModel: %1"_s.arg(QString::fromLatin1(name)));
        require(model->thread() == qApp->thread(), u"Model has the wrong thread affinity"_s);
        return model;
    }

    void exercise(QPointer<QMainWindow> window, QPointer<Content::ContentView> view)
    {
        require(window && view, u"Window closed before the scenario started"_s);
        waitFor([view]
        {
            return !view || (view->status() == QQuickWidget::Ready) || (view->status() == QQuickWidget::Error);
        }, 10'000, u"QML load"_s);
        require(view, u"Window closed before the scenario completed"_s);
        require(view->status() == QQuickWidget::Ready, u"Production ContentView did not load"_s);
        require(view->rootObject(), u"ContentView has no root object"_s);

        QPointer<Qbutt::ContentController> controller = view->findChild<Qbutt::ContentController *>();
        require(controller, u"Production ContentController is missing"_s);
        require(controller->thread() == qApp->thread(), u"Controller has the wrong thread affinity"_s);
        QPointer<QQuickItem> emptyResults = view->rootObject()->findChild<QQuickItem *>(u"contentEmptyResults"_s);
        require(emptyResults && !emptyResults->isVisible()
            && (controller->property("searchState").value<Qbutt::Search::SearchState>() == Qbutt::Search::SearchState::Idle),
            u"Idle search displayed a successful empty result"_s);
        QPointer<QAbstractItemModel> works = modelProperty(controller, "works");
        QPointer<QAbstractItemModel> variants = modelProperty(controller, "variants");
        require(works != variants, u"Work and variant collections share one model"_s);
        Qbutt::ContentController startCancelled;
        QAbstractItemModel *cancelledWorks = modelProperty(&startCancelled, "works");

        QObject observation;
        std::atomic_bool wrongThread = false;
        int modelChanges = 0;
        const auto changed = [&]
        {
            if (QThread::currentThread() != qApp->thread())
                wrongThread.store(true);
            else
                ++modelChanges;
        };
        for (QAbstractItemModel *model : {works.data(), variants.data(), cancelledWorks})
        {
            QObject::connect(model, &QAbstractItemModel::rowsInserted, &observation, changed, Qt::DirectConnection);
            QObject::connect(model, &QAbstractItemModel::rowsRemoved, &observation, changed, Qt::DirectConnection);
            QObject::connect(model, &QAbstractItemModel::modelReset, &observation, changed, Qt::DirectConnection);
            QObject::connect(model, &QAbstractItemModel::dataChanged, &observation, changed, Qt::DirectConnection);
        }

        bool lateStartupMutation = false;
        const auto changedAfterStartCancellation = [&]
        {
            if (QThread::currentThread() != qApp->thread())
                wrongThread.store(true);
            else
                lateStartupMutation = true;
        };
        QObject::connect(cancelledWorks, &QAbstractItemModel::rowsInserted, &observation, changedAfterStartCancellation, Qt::DirectConnection);
        QObject::connect(cancelledWorks, &QAbstractItemModel::rowsRemoved, &observation, changedAfterStartCancellation, Qt::DirectConnection);
        QObject::connect(cancelledWorks, &QAbstractItemModel::modelReset, &observation, changedAfterStartCancellation, Qt::DirectConnection);
        QObject::connect(cancelledWorks, &QAbstractItemModel::dataChanged, &observation, changedAfterStartCancellation, Qt::DirectConnection);
        QObject::connect(&startCancelled, &Qbutt::ContentController::errorTextChanged, &observation, changedAfterStartCancellation, Qt::DirectConnection);
        int startBusyNotifications = 0;
        QObject::connect(&startCancelled, &Qbutt::ContentController::busyChanged, &observation, [&]
        {
            if (QThread::currentThread() != qApp->thread())
            {
                wrongThread.store(true);
                return;
            }
            ++startBusyNotifications;
            if (startCancelled.property("busy").toBool())
                startCancelled.cancelSearch();
        }, Qt::DirectConnection);
        startCancelled.search(u"Naruto"_s);
        require((startBusyNotifications == 2) && !startCancelled.property("busy").toBool(),
            u"Cancellation inside the starting notification did not stop search"_s);

        controller->search(u"Naruto"_s);
        require(controller->property("busy").toBool(), u"Search did not enter its busy state"_s);
        controller->cancelSearch();
        require(!controller->property("busy").toBool(), u"Cancel did not leave the busy state"_s);
        require(!emptyResults->isVisible()
            && (controller->property("searchState").value<Qbutt::Search::SearchState>() == Qbutt::Search::SearchState::Cancelled),
            u"Cancelled search displayed a successful empty result"_s);

        bool changedQueryFromRows = false;
        bool replacementQueryStarted = false;
        const auto restartConnection = QObject::connect(works, &QAbstractItemModel::rowsInserted, &observation, [&]
        {
            if (changedQueryFromRows || (QThread::currentThread() != qApp->thread()))
                return;
            changedQueryFromRows = true;
            controller->search(u"Frieren"_s);
            replacementQueryStarted = controller->property("busy").toBool();
        }, Qt::DirectConnection);
        controller->search(u"Naruto"_s);
        require(controller->property("busy").toBool(), u"New search did not start after cancellation"_s);
        waitFor([&] { return !controller || !controller->property("busy").toBool(); }, 100'000, u"Live AniLiberty search"_s);
        QObject::disconnect(restartConnection);
        require(controller, u"Window closed during search"_s);
        require(changedQueryFromRows && replacementQueryStarted, u"Synchronous result notification did not start the replacement query"_s);
        require(controller->property("errorText").toString().isEmpty(),
            u"Search failed: %1"_s.arg(controller->property("errorText").toString()));
        require(!wrongThread.load(), u"Model mutation was emitted outside the GUI thread"_s);
        require((works->rowCount() > 0) && (works->rowCount() <= 500), u"Search returned an invalid row count"_s);
        require(!emptyResults->isVisible()
            && (controller->property("searchState").value<Qbutt::Search::SearchState>() == Qbutt::Search::SearchState::Succeeded),
            u"Completed search did not publish its successful outcome"_s);

        QSet<QString> identities;
        QString selectedWork;
        QString selectedTitle;
        QString otherWork;
        role(works, "year");
        for (int row = 0; row < works->rowCount(); ++row)
        {
            const QString id = value(works, row, "workId");
            const QString title = value(works, row, "title");
            require(!id.isEmpty() && !identities.contains(id), u"Work identity is missing or duplicated"_s);
            require(!title.isEmpty(), u"Search returned an unnamed work"_s);
            const QString source = value(works, row, "source");
            const QString category = value(works, row, "category");
            require(((source == u"AniLiberty") && (category == u"anime"))
                || ((source == u"MangaDex") && (category == u"manga")),
                u"Incorrect work source or category role"_s);
            if (source == u"AniLiberty")
            {
                require(title.contains(u"Frieren"_s, Qt::CaseInsensitive) || title.contains(u"Фрирен"_s, Qt::CaseInsensitive),
                    u"Cancelled or unrelated search results entered the new search"_s);
            }
            identities.insert(id);
            if (id == u"aniliberty:9542")
            {
                selectedWork = id;
                selectedTitle = title;
            }
            else if (source == u"AniLiberty")
                otherWork = id;
        }
        require(!selectedWork.isEmpty(), u"Live catalog did not return the expected publication 9542"_s);
        require(!otherWork.isEmpty(), u"Live catalog needs a second publication for selection replacement"_s);

        QPointer<QObject> resultView = view->rootObject()->findChild<QObject *>(u"contentResults"_s);
        require(resultView, u"The production results view is missing"_s);
        waitFor([&] { return !resultView || (resultView->property("count").toInt() == works->rowCount()); }, 2'000, u"Work model binding"_s);
        require(resultView, u"Window closed before work model binding"_s);

        bool replacedFromIdentity = false;
        QString replacementDescription;
        const auto identityConnection = QObject::connect(controller, &Qbutt::ContentController::selectedWorkIdChanged, &observation, [&]
        {
            if (QThread::currentThread() != qApp->thread())
            {
                wrongThread.store(true);
                return;
            }
            if (replacedFromIdentity || (controller->property("selectedWorkId").toString() != otherWork))
                return;
            replacedFromIdentity = true;
            controller->selectWork(selectedWork);
            replacementDescription = controller->property("selectedWorkDescription").toString();
        }, Qt::DirectConnection);
        controller->selectWork(otherWork);
        QObject::disconnect(identityConnection);
        require(replacedFromIdentity && (controller->property("selectedWorkId").toString() == selectedWork)
            && (controller->property("selectedWorkTitle").toString() == selectedTitle)
            && (controller->property("selectedWorkDescription").toString() == replacementDescription),
            u"Interrupted selection overwrote replacement identity or metadata"_s);
        waitFor([&] { return !controller || !controller->property("inspecting").toBool(); }, 50'000, u"Publication inspection"_s);
        require(controller, u"Window closed during inspection"_s);
        require(controller->property("errorText").toString().isEmpty(),
            u"Inspection failed: %1"_s.arg(controller->property("errorText").toString()));
        require(controller->property("selectedWorkId").toString() == selectedWork, u"Inspection changed publication identity"_s);
        require(!controller->property("selectedWorkTitle").toString().isEmpty(), u"Inspected title is missing"_s);
        require(variants->rowCount() > 0, u"Inspection did not expose torrent variants"_s);
        const QString inspectedTitle = controller->property("selectedWorkTitle").toString();
        const QString inspectedDescription = controller->property("selectedWorkDescription").toString();

        bool replacedFromInspection = false;
        const auto inspectionConnection = QObject::connect(controller, &Qbutt::ContentController::inspectingChanged, &observation, [&]
        {
            if (QThread::currentThread() != qApp->thread())
            {
                wrongThread.store(true);
                return;
            }
            if (replacedFromInspection || !controller->property("inspecting").toBool())
                return;
            replacedFromInspection = true;
            controller->selectWork(selectedWork);
        }, Qt::DirectConnection);
        controller->selectWork(otherWork);
        QObject::disconnect(inspectionConnection);
        require(replacedFromInspection && !controller->property("inspecting").toBool()
            && (controller->property("selectedWorkId").toString() == selectedWork)
            && (controller->property("selectedWorkTitle").toString() == inspectedTitle)
            && (controller->property("selectedWorkDescription").toString() == inspectedDescription)
            && controller->property("errorText").toString().isEmpty(),
            u"Replacement inside the starting inspection notification did not take ownership"_s);

        bool lateInspectionMutation = false;
        const auto changedAfterReplacement = [&]
        {
            if (QThread::currentThread() != qApp->thread())
                wrongThread.store(true);
            else
                lateInspectionMutation = true;
        };
        const QList<QMetaObject::Connection> inspectionMutations {
            QObject::connect(works, &QAbstractItemModel::dataChanged, &observation, changedAfterReplacement, Qt::DirectConnection),
            QObject::connect(variants, &QAbstractItemModel::dataChanged, &observation, changedAfterReplacement, Qt::DirectConnection),
            QObject::connect(variants, &QAbstractItemModel::modelReset, &observation, changedAfterReplacement, Qt::DirectConnection),
            QObject::connect(variants, &QAbstractItemModel::rowsInserted, &observation, changedAfterReplacement, Qt::DirectConnection),
            QObject::connect(variants, &QAbstractItemModel::rowsRemoved, &observation, changedAfterReplacement, Qt::DirectConnection),
            QObject::connect(controller, &Qbutt::ContentController::errorTextChanged, &observation, changedAfterReplacement, Qt::DirectConnection),
            QObject::connect(controller, &Qbutt::ContentController::selectedWorkIdChanged, &observation, changedAfterReplacement, Qt::DirectConnection),
            QObject::connect(controller, &Qbutt::ContentController::selectedWorkTitleChanged, &observation, changedAfterReplacement, Qt::DirectConnection),
            QObject::connect(controller, &Qbutt::ContentController::selectedWorkDescriptionChanged, &observation, changedAfterReplacement, Qt::DirectConnection),
            QObject::connect(controller, &Qbutt::ContentController::inspectingChanged, &observation, changedAfterReplacement, Qt::DirectConnection)
        };
        QElapsedTimer cancelledOperationAge;
        cancelledOperationAge.start();

        identities.clear();
        for (const QByteArray &name : {QByteArray("quality"), QByteArray("sizeText"), QByteArray("seedersText"), QByteArray("infoHash")})
            role(variants, name);
        for (int row = 0; row < variants->rowCount(); ++row)
        {
            const QString id = value(variants, row, "variantId");
            require(id.startsWith(selectedWork + u":torrent:"_s) && !identities.contains(id),
                u"Variant identity is duplicated or belongs to another publication"_s);
            require(!value(variants, row, "label").isEmpty(), u"Torrent variant has no label"_s);
            require(value(variants, row, "source") == u"AniLiberty", u"Incorrect variant source role"_s);
            require(value(variants, row, "kind") == u"torrent", u"An unsupported action entered the torrent slice"_s);
            identities.insert(id);
        }

        QPointer<QQuickItem> variantView = view->rootObject()->findChild<QQuickItem *>(u"contentVariants"_s);
        require(variantView, u"The production variants view is missing"_s);
        waitFor([&] { return !variantView || (variantView->property("count").toInt() == variants->rowCount()); }, 2'000, u"Variant model binding"_s);
        require(variantView, u"Window closed before variant model binding"_s);

        variantView->forceActiveFocus();
        QKeyEvent selectLastPress(QEvent::KeyPress, Qt::Key_End, Qt::NoModifier);
        QCoreApplication::sendEvent(view, &selectLastPress);
        QKeyEvent selectLastRelease(QEvent::KeyRelease, Qt::Key_End, Qt::NoModifier);
        QCoreApplication::sendEvent(view, &selectLastRelease);
        const int selectedRow = variants->rowCount() - 1;
        const QString selectedVariant = value(variants, selectedRow, "variantId");
        require((variantView->property("currentIndex").toInt() == selectedRow)
            && (controller->property("selectedVariantId").toString() == selectedVariant)
            && (controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
                == Qbutt::Search::VariantSelectionState::Available),
            u"Keyboard variant highlight and exact selection differ"_s);
        const QString expectedHash = value(variants, selectedRow, "infoHash").toLower();
        require(expectedHash.size() == 40, u"Selected torrent has no verifiable infohash"_s);
        QString receivedMagnet;
        int torrentSignals = 0;
        QObject::connect(view, &Content::ContentView::torrentReady, &observation, [&](const QString &magnet)
        {
            if (QThread::currentThread() != qApp->thread())
            {
                wrongThread.store(true);
                return;
            }
            receivedMagnet = magnet;
            ++torrentSignals;
        }, Qt::DirectConnection);
        QKeyEvent downloadPress(QEvent::KeyPress, Qt::Key_Return, Qt::NoModifier);
        QCoreApplication::sendEvent(view, &downloadPress);
        QKeyEvent downloadRelease(QEvent::KeyRelease, Qt::Key_Return, Qt::NoModifier);
        QCoreApplication::sendEvent(view, &downloadRelease);
        waitFor([&] { return !controller || !controller->property("resolving").toBool(); }, 50'000, u"Exact torrent resolution"_s);
        require(controller, u"Window closed during resolution"_s);
        require(controller->property("errorText").toString().isEmpty(),
            u"Resolution failed: %1"_s.arg(controller->property("errorText").toString()));
        require(torrentSignals == 1, u"Exact torrent resolution did not emit one native handoff"_s);
        const QUrl magnet(receivedMagnet);
        const QStringList hashes = QUrlQuery(magnet).allQueryItemValues(u"xt"_s);
        require((magnet.scheme() == u"magnet") && (hashes.size() == 1)
            && (hashes.first().compare(u"urn:btih:"_s + expectedHash, Qt::CaseInsensitive) == 0),
            u"Native handoff changed the selected torrent identity"_s);
        require((modelChanges > 0) && !wrongThread.load(), u"Model signal thread checks failed"_s);

        const auto cancellationViolated = [&]
        {
            return !controller || wrongThread.load() || (startBusyNotifications != 2)
                || (cancelledWorks->rowCount() != 0) || startCancelled.property("busy").toBool()
                || !startCancelled.property("errorText").toString().isEmpty() || lateStartupMutation || lateInspectionMutation
                || !controller->property("errorText").toString().isEmpty()
                || controller->property("inspecting").toBool()
                || (controller->property("selectedWorkId").toString() != selectedWork)
                || (controller->property("selectedWorkTitle").toString() != inspectedTitle)
                || (controller->property("selectedWorkDescription").toString() != inspectedDescription);
        };
        // The native HTTP operation is bounded to 45 seconds; allow its queued completion to settle.
        waitFor([&] { return cancellationViolated() || (cancelledOperationAge.elapsed() >= 46'000); },
            47'000, u"Cancelled startup operations"_s);
        require(!cancellationViolated(), u"An operation cancelled during startup published a late state change"_s);
        for (const auto &connection : inspectionMutations)
            QObject::disconnect(connection);

        const int workCount = works->rowCount();
        const int variantCount = variants->rowCount();

        bool cancelledFromResolution = false;
        const auto cancelConnection = QObject::connect(controller, &Qbutt::ContentController::resolvingChanged, &observation, [&]
        {
            if (QThread::currentThread() != qApp->thread())
            {
                wrongThread.store(true);
                return;
            }
            if (controller->property("resolving").toBool() || !controller->property("errorText").toString().isEmpty())
                return;
            cancelledFromResolution = true;
            controller->search(QString {});
        }, Qt::DirectConnection);
        controller->resolveVariant(selectedVariant);
        waitFor([&] { return !controller || !controller->property("resolving").toBool(); }, 50'000, u"Synchronous resolution cancellation"_s);
        QObject::disconnect(cancelConnection);
        require(controller, u"Window closed during resolution cancellation"_s);
        require(controller->property("errorText").toString().isEmpty(),
            u"Second resolution failed: %1"_s.arg(controller->property("errorText").toString()));
        require(cancelledFromResolution && (works->rowCount() == 0) && (torrentSignals == 1),
            u"Cancelled resolution emitted a stale torrent handoff"_s);
        require(!emptyResults->isVisible()
            && (controller->property("searchState").value<Qbutt::Search::SearchState>() == Qbutt::Search::SearchState::Idle),
            u"Cleared search retained a successful empty outcome"_s);

        controller->search(u"qbutt-acceptance-missing-title-71c03d90"_s);
        waitFor([&] { return !controller || !controller->property("busy").toBool(); }, 50'000, u"Successful empty search"_s);
        require(controller && emptyResults && controller->property("errorText").toString().isEmpty()
            && (works->rowCount() == 0) && emptyResults->isVisible()
            && (controller->property("searchState").value<Qbutt::Search::SearchState>() == Qbutt::Search::SearchState::Succeeded),
            u"Successful empty search did not show its empty result"_s);

        controller->search(u"а"_s);
        waitFor([&] { return !controller || !controller->property("busy").toBool(); }, 50'000, u"Broad catalog search"_s);
        require(controller && controller->property("errorText").toString().isEmpty() && (works->rowCount() > 5),
            u"Broad search did not produce a list for boundary navigation"_s);
        auto *results = qobject_cast<QQuickItem *>(resultView.data());
        require(results, u"Results list is missing"_s);
        results->forceActiveFocus();
        for (const auto key : {Qt::Key_End, Qt::Key_Home})
        {
            QKeyEvent press(QEvent::KeyPress, key, Qt::NoModifier);
            QCoreApplication::sendEvent(view, &press);
            QKeyEvent release(QEvent::KeyRelease, key, Qt::NoModifier);
            QCoreApplication::sendEvent(view, &release);
            const int expectedRow = (key == Qt::Key_Home) ? 0 : works->rowCount() - 1;
            const QString expectedWork = value(works, expectedRow, "workId");
            waitFor([&]
            {
                return controller->property("selectedWorkId").toString() == expectedWork;
            }, 5000, u"Keyboard boundary selection"_s);
            require(results->property("currentIndex").toInt() == expectedRow,
                u"Keyboard boundary selection moved to the wrong row"_s);
        }

        controller->search(u"One Piece"_s);
        require(controller->property("busy").toBool(), u"Shutdown scenario has no pending request"_s);
        bool closedWithPendingSearch = false;
        QTimer::singleShot(0, window, [&]
        {
            closedWithPendingSearch = controller && controller->property("busy").toBool();
            window->close();
        });
        waitFor([&] { return controller.isNull() && works.isNull() && variants.isNull(); }, 5'000, u"Close during search"_s);
        require(closedWithPendingSearch, u"Search completed before the shutdown scenario could close the window"_s);

        // Let already queued GUI callbacks run after their targets are deleted.
        QEventLoop drain;
        QTimer::singleShot(500, &drain, &QEventLoop::quit);
        drain.exec();
        require(!wrongThread.load(), u"A late model callback used the wrong thread"_s);
        qInfo().noquote() << u"PASS works=%1 variants=%2 search_outcomes=ok boundary_keys=ok variant_keyboard=ok cancel_restart=ok startup_cancel=ok exact_magnet=ok reentrant_cancel=ok close_pending=ok"_s
            .arg(workCount).arg(variantCount);
    }
}

int main(int argc, char *argv[])
{
    // Keep the fixture until QApplication has drained the owned workers.
    std::unique_ptr<QTemporaryDir> temporaryProfile;
    QApplication application(argc, argv);
    QCoreApplication::setApplicationName(u"qbutt-quick-acceptance"_s);
    QCoreApplication::setOrganizationName(u"qbutt-acceptance"_s);
    QStandardPaths::setTestModeEnabled(true);

    QCommandLineParser parser;
    parser.setApplicationDescription(u"Runs the production Qt Quick content view with an isolated application identity."_s);
    parser.addHelpOption();
    parser.addOption({u"exercise"_s, u"Run the bounded live metadata acceptance scenario and exit."_s});
    parser.addOption({u"library"_s, u"Run native library persistence and fault scenarios and exit."_s});
    parser.addOption({u"history"_s, u"Run confirmed reading history and SQLite fault scenarios, then exit."_s});
    parser.addOption({u"reader"_s, u"Run live manga search and reading acceptance, then exit."_s});
    parser.addOption({u"reader-progress"_s, u"Run exact reader progress and SQLite fault scenarios, then exit."_s});
    parser.addOption({u"reader-settings"_s, u"Run reader settings persistence and SQLite faults, then exit."_s});
    parser.addOption({u"reader-settings-phase"_s, u"Run a reader settings restart fixture phase."_s, u"phase"_s});
    parser.addOption({u"worklist"_s, u"Run work list delegate and favorites acceptance, then exit."_s});
    parser.addOption({u"profile"_s, u"Use an explicit isolated profile for restart acceptance."_s, u"directory"_s});
    parser.addOption({u"dark"_s, u"Use the dark system color scheme for visual inspection."_s});
    parser.addOption({u"lang"_s, u"Set the locale for this process."_s, u"locale"_s, u"en"_s});
    parser.process(application);

    QLocale::setDefault(QLocale(parser.value(u"lang"_s)));
    QTranslator translator;
    if ((QLocale().language() == QLocale::Russian)
        && translator.load(u":/lang/qbittorrent_ru.qm"_s))
    {
        application.installTranslator(&translator);
    }
    QGuiApplication::styleHints()->setColorScheme(parser.isSet(u"dark"_s) ? Qt::ColorScheme::Dark : Qt::ColorScheme::Light);
    application.setQuitOnLastWindowClosed(!parser.isSet(u"exercise"_s)
        && !parser.isSet(u"reader"_s) && !parser.isSet(u"worklist"_s));

    if (parser.isSet(u"library"_s) || parser.isSet(u"reader-progress"_s) || parser.isSet(u"history"_s)
        || parser.isSet(u"reader-settings"_s))
    {
        try
        {
            if (parser.isSet(u"reader-settings"_s))
                exerciseReaderSettings(parser.value(u"profile"_s), parser.value(u"reader-settings-phase"_s));
            else if (parser.isSet(u"history"_s))
                exerciseHistory();
            else if (parser.isSet(u"reader-progress"_s))
                exerciseReaderProgress();
            else
                exerciseLibrary();
            return 0;
        }
        catch (const std::exception &error)
        {
            qCritical().noquote() << "FAIL" << error.what();
            return 1;
        }
    }

    QString profilePath = parser.value(u"profile"_s);
    if (profilePath.isEmpty())
    {
        temporaryProfile = std::make_unique<QTemporaryDir>();
        if (!temporaryProfile->isValid())
            return 1;
        profilePath = temporaryProfile->path();
    }
    else if (!QDir::isAbsolutePath(profilePath))
    {
        qCritical("Acceptance profile must be an absolute isolated directory");
        return 1;
    }
    QPointer<QMainWindow> window = new QMainWindow;
    window->setAttribute(Qt::WA_DeleteOnClose);
    window->setWindowTitle(u"qbutt — Qt Quick acceptance"_s);
    window->resize(1280, 760);
    auto *view = new Content::ContentView(profilePath, window);
    window->setCentralWidget(view);
    QObject::connect(view, &Content::ContentView::torrentReady, window, [window](const QString &magnet)
    {
        window->statusBar()->showMessage(u"Torrent handoff received; this acceptance host does not download files."_s);
        qInfo().noquote() << u"TORRENT_HANDOFF"_s << QUrlQuery(QUrl(magnet)).queryItemValue(u"xt"_s);
    });
    window->show();

    if (parser.isSet(u"exercise"_s) || parser.isSet(u"reader"_s) || parser.isSet(u"worklist"_s))
    {
        QTimer::singleShot(360'000, &application, [&application]
        {
            qCritical("Acceptance exceeded its overall time limit");
            application.exit(1);
        });
        QTimer::singleShot(0, &application,
            [&application, window = QPointer(window), view = QPointer(view),
                runReader = parser.isSet(u"reader"_s), runWorkList = parser.isSet(u"worklist"_s)]
        {
            try
            {
                if (runWorkList)
                    exerciseWorkList(view);
                else if (runReader)
                    exerciseReader(view);
                else
                    exercise(window, view);
                application.exit(0);
            }
            catch (const std::exception &error)
            {
                qCritical().noquote() << "FAIL" << error.what();
                application.exit(1);
            }
        });
    }

    const int result = application.exec();
    delete window.data();
    return result;
}
