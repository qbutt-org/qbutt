// SPDX-License-Identifier: GPL-2.0-or-later
#include "sources.h"

#include <functional>
#include <memory>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QDebug>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QJsonDocument>
#include <QJsonObject>
#include <QKeyEvent>
#include <QQuickItem>
#include <QScopeGuard>
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QTemporaryDir>
#include <QThreadPool>
#include <QTimer>

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

    void waitFor(const std::function<bool()> &ready, const QString &operation, const int timeout = 10'000)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < timeout))
        {
            QEventLoop loop;
            QTimer::singleShot(20, &loop, &QEventLoop::quit);
            loop.exec();
        }
        require(ready(), operation + u" timed out"_s);
    }

    QAbstractItemModel *model(Qbutt::ContentController &controller, const char *name)
    {
        auto *result = qobject_cast<QAbstractItemModel *>(controller.property(name).value<QObject *>());
        require(result && (result->thread() == qApp->thread()), u"Invalid typed source model"_s);
        return result;
    }

    QVariant value(QAbstractItemModel *model, const QString &id, const QByteArray &name)
    {
        const auto roles = model->roleNames();
        require(roles.contains(roles.key(name, -1)), u"Missing source role"_s);
        for (int row = 0; row < model->rowCount(); ++row)
        {
            if (model->data(model->index(row, 0), roles.key("sourceId")).toString() == id)
                return model->data(model->index(row, 0), roles.key(name));
        }
        throw std::runtime_error("Missing source row");
    }

    QQuickItem *visualItem(QQuickItem *parent, const QString &name)
    {
        if (!parent || (parent->objectName() == name))
            return parent;
        for (auto *child : parent->childItems())
        {
            if (auto *item = visualItem(child, name))
                return item;
        }
        return nullptr;
    }

    auto state(Qbutt::ContentController &controller)
    {
        return controller.property("sourceSettingsState").value<Qbutt::Sources::SettingsState>();
    }

    void loaded(Qbutt::ContentController &controller)
    {
        waitFor([&] { return state(controller) != Qbutt::Sources::SettingsState::Loading; }, u"Source snapshot"_s);
        waitFor([&]
        {
            return controller.property("libraryState").value<Qbutt::Library::LibraryState>() != Qbutt::Library::LibraryState::Loading;
        }, u"Library snapshot"_s);
    }

    void saved(Qbutt::ContentController &controller)
    {
        waitFor([&] { return !controller.property("libraryPending").toBool(); }, u"Source SQL acknowledgement"_s);
        require(controller.property("libraryErrorText").toString().isEmpty(), u"Source writes polluted the library error"_s);
        require(controller.property("errorText").toString().isEmpty(), u"Source errors polluted search"_s);
    }

    void exec(QSqlDatabase &database, const QString &sql)
    {
        QSqlQuery query(database);
        require(query.exec(sql), query.lastError().text());
    }

    QJsonObject inspection(const QString &id)
    {
        return {{u"inspect"_s, QJsonObject {
            {u"reference"_s, QJsonObject {{u"source"_s, u"ani_liberty"_s}, {u"id"_s, id}}},
            {u"language"_s, QJsonValue::Null}}}};
    }

    QJsonObject searchOperation()
    {
        return {{u"search"_s, QJsonObject {{u"source"_s, u"ani_liberty"_s},
            {u"query"_s, u"Frieren"_s}, {u"category"_s, u"anime"_s}, {u"language"_s, QJsonValue::Null}}}};
    }

    void failure(QSqlDatabase &database, const QJsonObject &operation)
    {
        QSqlQuery query(database);
        query.prepare(u"UPDATE source_settings SET failure = ?, revision = revision + 1 WHERE source = 'aniliberty'"_s);
        query.addBindValue(QString::fromUtf8(QJsonDocument(QJsonObject {
            {u"operation"_s, operation}, {u"reason"_s, u"timeout"_s}}).toJson(QJsonDocument::Compact)));
        require(query.exec() && (query.numRowsAffected() == 1), query.lastError().text());
    }

    QJsonObject persistedFailure(QSqlDatabase &database)
    {
        QSqlQuery query(database);
        require(query.exec(u"SELECT failure FROM source_settings WHERE source = 'aniliberty'"_s) && query.next(), query.lastError().text());
        return QJsonDocument::fromJson(query.value(0).toString().toUtf8()).object();
    }
}

void exerciseSources()
{
    require(QGuiApplication::platformName() == u"offscreen"_s, u"Sources process fixture requires offscreen Qt"_s);
    QTemporaryDir profile;
    require(profile.isValid(), u"Owned source profile failed"_s);
    const auto drain = qScopeGuard([]
    {
        if (!QThreadPool::globalInstance()->waitForDone(100'000))
            qFatal("Source owners did not drain");
    });

    auto controller = std::make_unique<Qbutt::ContentController>();
    require(state(*controller) == Qbutt::Sources::SettingsState::Ready
        && (model(*controller, "sources")->rowCount() == 3), u"Ephemeral source catalog is not ready"_s);
    bool reenteredInit = false;
    QObject initObserver;
    const auto initConnection = QObject::connect(controller.get(), &Qbutt::ContentController::sourcesChanged, &initObserver, [&]
    {
        if (!reenteredInit && (state(*controller) == Qbutt::Sources::SettingsState::Loading))
        {
            reenteredInit = true;
            controller->initLibrary(u"relative-reentrant-profile"_s);
        }
    }, Qt::DirectConnection);
    controller->initLibrary(profile.path());
    require(state(*controller) == Qbutt::Sources::SettingsState::Loading
        && (model(*controller, "sources")->rowCount() == 0)
        && !controller->setSourceEnabled(u"rutor"_s, false), u"Initial settings were treated as defaults"_s);
    loaded(*controller);
    QObject::disconnect(initConnection);
    require(reenteredInit && (controller->property("libraryState").value<Qbutt::Library::LibraryState>() == Qbutt::Library::LibraryState::Ready),
        u"Reentrant initialization replaced the owned profile/worker"_s);
    for (const QString &id : {u"aniliberty"_s, u"mangadex"_s, u"rutor"_s})
        require(controller->setSourceEnabled(id, false), u"Source disable rejected"_s);
    saved(*controller);
    controller.reset();
    require(QThreadPool::globalInstance()->waitForDone(100'000), u"First source connection did not drain"_s);

    const auto unregister = qScopeGuard([] { QSqlDatabase::removeDatabase(u"source-faults"_s); });
    QSqlDatabase database = QSqlDatabase::addDatabase(u"QSQLITE"_s, u"source-faults"_s);
    database.setDatabaseName(QDir(profile.path()).filePath(u"content-library.sqlite"_s));
    require(database.open(), database.lastError().text());

    controller = std::make_unique<Qbutt::ContentController>();
    controller->initLibrary(profile.path());
    controller->search(u"Frieren"_s);
    require(controller->property("busy").toBool() && (model(*controller, "works")->rowCount() == 0), u"Search did not wait for source settings"_s);
    loaded(*controller);
    waitFor([&] { return !controller->property("busy").toBool(); }, u"Deferred disabled search"_s);
    require(model(*controller, "works")->rowCount() == 0, u"Disabled source was queried before persisted settings loaded"_s);

    exec(database, u"CREATE TRIGGER reject_source BEFORE UPDATE ON source_settings BEGIN SELECT RAISE(ABORT, 'generated source write fault'); END;"_s);
    require(controller->setSourceEnabled(u"mangadex"_s, true), u"Source intent was not accepted"_s);
    saved(*controller);
    require(state(*controller) == Qbutt::Sources::SettingsState::Failed
        && value(model(*controller, "sources"), u"mangadex"_s, "enabled").toBool()
        && !controller->property("sourceSettingsErrorText").toString().isEmpty(), u"Failed SQL lost the chosen source intent"_s);
    require(controller->refreshHistory(), u"History read rejected after a source write error"_s);
    require(controller->setReaderFit(Qbutt::Reader::ReaderFit::Width), u"Reader settings rejected after source error"_s);
    saved(*controller);
    require(state(*controller) == Qbutt::Sources::SettingsState::Failed, u"History/reader success cleared the source write error"_s);
    require(controller->setSourceEnabled(u"aniliberty"_s, true), u"Second source intent rejected"_s);
    saved(*controller);
    exec(database, u"DROP TRIGGER reject_source"_s);
    bool changedDuringRetry = false;
    QObject retryObserver;
    const auto retryConnection = QObject::connect(controller.get(), &Qbutt::ContentController::sourcesChanged, &retryObserver, [&]
    {
        if (!changedDuringRetry && value(model(*controller, "sources"), u"aniliberty"_s, "pending").toBool())
        {
            changedDuringRetry = true;
            require(controller->setSourceEnabled(u"mangadex"_s, false), u"New choice during batch retry rejected"_s);
        }
    }, Qt::DirectConnection);
    require(controller->retrySourceSettings(), u"Exact source SQL retry rejected"_s);
    saved(*controller);
    QObject::disconnect(retryConnection);
    require(state(*controller) == Qbutt::Sources::SettingsState::Ready, u"Source SQL retry did not recover"_s);
    require(changedDuringRetry && !value(model(*controller, "sources"), u"mangadex"_s, "enabled").toBool(),
        u"Batch retry replayed a superseded source choice"_s);

    bool reentered = false;
    QObject observer;
    const auto connection = QObject::connect(controller.get(), &Qbutt::ContentController::sourcesChanged, &observer, [&]
    {
        if (!reentered && value(model(*controller, "sources"), u"rutor"_s, "enabled").toBool())
        {
            reentered = true;
            require(controller->setSourceEnabled(u"rutor"_s, false), u"Reentrant source choice rejected"_s);
        }
    }, Qt::DirectConnection);
    require(controller->setSourceEnabled(u"rutor"_s, true), u"First reentrant choice rejected"_s);
    saved(*controller);
    QObject::disconnect(connection);
    require(reentered && !value(model(*controller, "sources"), u"rutor"_s, "enabled").toBool(), u"Older source acknowledgement overwrote the newer choice"_s);
    controller.reset();
    require(QThreadPool::globalInstance()->waitForDone(100'000), u"Second source connection did not drain"_s);

    exec(database, u"PRAGMA ignore_check_constraints = ON"_s);
    exec(database, u"UPDATE reader_settings SET fit = 77 WHERE id = 1"_s);
    exec(database, u"PRAGMA ignore_check_constraints = OFF"_s);
    controller = std::make_unique<Qbutt::ContentController>();
    bool acceptedDuringSnapshotFailure = false;
    QObject failedSnapshotObserver;
    const auto snapshotConnection = QObject::connect(controller.get(), &Qbutt::ContentController::sourcesChanged, &failedSnapshotObserver, [&]
    {
        if (!acceptedDuringSnapshotFailure && (state(*controller) == Qbutt::Sources::SettingsState::Ready))
        {
            acceptedDuringSnapshotFailure = controller->setSourceEnabled(u"aniliberty"_s, false);
        }
    }, Qt::DirectConnection);
    controller->initLibrary(profile.path());
    loaded(*controller);
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Accepted source write after snapshot failure"_s);
    QObject::disconnect(snapshotConnection);
    require(acceptedDuringSnapshotFailure && (state(*controller) == Qbutt::Sources::SettingsState::Ready)
        && (controller->property("libraryState").value<Qbutt::Library::LibraryState>() == Qbutt::Library::LibraryState::Failed)
        && !value(model(*controller, "sources"), u"aniliberty"_s, "enabled").toBool(), u"Snapshot failure lost an accepted FIFO source write/acknowledgement"_s);
    controller.reset();
    require(QThreadPool::globalInstance()->waitForDone(100'000), u"Failed snapshot connection did not drain"_s);
    exec(database, u"UPDATE reader_settings SET fit = 1 WHERE id = 1"_s);

    exec(database, u"UPDATE source_settings SET failure = 'broken JSON' WHERE source = 'rutor'"_s);
    controller = std::make_unique<Qbutt::ContentController>();
    controller->initLibrary(profile.path());
    loaded(*controller);
    require(state(*controller) == Qbutt::Sources::SettingsState::Failed
        && (model(*controller, "sources")->rowCount() == 0)
        && (controller->property("libraryState").value<Qbutt::Library::LibraryState>() == Qbutt::Library::LibraryState::Ready)
        && (controller->property("readerFit").value<Qbutt::Reader::ReaderFit>() == Qbutt::Reader::ReaderFit::Width),
        u"Corrupt source snapshot disabled the library/reader or displayed unconfirmed defaults"_s);
    controller->search(u"Frieren"_s);
    require(!controller->property("busy").toBool() && (model(*controller, "works")->rowCount() == 0)
        && !controller->setSourceEnabled(u"rutor"_s, true), u"Unconfirmed source settings became actionable"_s);
    exec(database, u"UPDATE source_settings SET failure = NULL WHERE source = 'rutor'"_s);
    require(controller->retrySourceSettings(), u"Source snapshot retry rejected"_s);
    saved(*controller);
    require(state(*controller) == Qbutt::Sources::SettingsState::Ready
        && (model(*controller, "sources")->rowCount() == 3), u"Source snapshot did not recover"_s);
    controller.reset();
    require(QThreadPool::globalInstance()->waitForDone(100'000), u"Snapshot retry connection did not drain"_s);

    exec(database, u"UPDATE source_settings SET enabled = (source = 'aniliberty')"_s);
    failure(database, inspection(u"9542"_s));
    controller = std::make_unique<Qbutt::ContentController>();
    controller->initLibrary(profile.path());
    loaded(*controller);
    int handoffs = 0;
    QObject::connect(controller.get(), &Qbutt::ContentController::torrentReady, &observer, [&] { ++handoffs; });
    require(value(model(*controller, "sources"), u"aniliberty"_s, "failed").toBool()
        && controller->property("errorText").toString().isEmpty(), u"Persisted source failure did not remain confined to Sources"_s);
    require(controller->retrySource(u"aniliberty"_s), u"Actual inspect retry rejected"_s);
    // The HTTP job is live, but its callback cannot run until this newer SQL revision is installed.
    failure(database, searchOperation());
    waitFor([&] { return !value(model(*controller, "sources"), u"aniliberty"_s, "retrying").toBool(); }, u"Late inspect retry"_s, 50'000);
    saved(*controller);
    require(persistedFailure(database).value(u"operation"_s).toObject() == searchOperation()
        && value(model(*controller, "sources"), u"aniliberty"_s, "failed").toBool(), u"Late success cleared a different failure revision"_s);
    require(controller->retrySource(u"aniliberty"_s), u"Current exact inspect retry rejected"_s);
    waitFor([&] { return !value(model(*controller, "sources"), u"aniliberty"_s, "retrying").toBool(); }, u"Exact inspect retry"_s, 50'000);
    saved(*controller);
    require(!value(model(*controller, "sources"), u"aniliberty"_s, "failed").toBool()
        && persistedFailure(database).isEmpty() && (handoffs == 0)
        && controller->property("selectedWorkId").toString().isEmpty(), u"Source retry did not clear only its failure or changed the selected work/handoff"_s);

    controller.reset();
    require(QThreadPool::globalInstance()->waitForDone(100'000), u"Source retry connection did not drain"_s);
    failure(database, inspection(u"18446744073709551615"_s));
    controller = std::make_unique<Qbutt::ContentController>();
    controller->initLibrary(profile.path());
    loaded(*controller);
    require(controller->retrySource(u"aniliberty"_s), u"Missing publication retry rejected"_s);
    waitFor([&] { return !value(model(*controller, "sources"), u"aniliberty"_s, "retrying").toBool(); }, u"Actual missing publication response"_s, 50'000);
    saved(*controller);
    require(value(model(*controller, "sources"), u"aniliberty"_s, "failed").toBool()
        && (persistedFailure(database).value(u"operation"_s).toObject() == inspection(u"18446744073709551615"_s))
        && (persistedFailure(database).value(u"reason"_s) != QJsonValue(u"timeout"_s)), u"Actual source failure was not recorded with its exact public operation"_s);
    controller.reset();
    require(QThreadPool::globalInstance()->waitForDone(100'000), u"Missing publication connection did not drain"_s);

    Content::ContentView view(profile.path());
    view.resize(1440, 900);
    view.show();
    auto *uiController = view.findChild<Qbutt::ContentController *>();
    require(uiController, u"Production view has no Sources controller"_s);
    loaded(*uiController);
    waitFor([&] { return view.status() != QQuickWidget::Loading; }, u"Production Sources QML"_s);
    require(view.status() == QQuickWidget::Ready, u"Sources QML did not load"_s);
    uiController->search(u"Frieren"_s);
    waitFor([&] { return !uiController->property("busy").toBool(); }, u"Source UI search context"_s, 50'000);
    uiController->selectWork(u"aniliberty:9542"_s);
    waitFor([&] { return !uiController->property("inspecting").toBool(); }, u"Source UI exact selection"_s, 50'000);
    saved(*uiController);
    const int workCount = model(*uiController, "works")->rowCount();
    const QString selectedWork = uiController->property("selectedWorkId").toString();
    const QString selectedVariant = uiController->property("selectedVariantId").toString();
    require((workCount > 0) && (selectedWork == u"aniliberty:9542"_s) && !selectedVariant.isEmpty(), u"Source UI context has no real exact selection"_s);
    auto *sourceButton = view.rootObject()->findChild<QQuickItem *>(u"contentSources"_s);
    require(sourceButton && QMetaObject::invokeMethod(sourceButton, "clicked"), u"Production Sources action failed"_s);
    QObject *picker = nullptr;
    waitFor([&]
    {
        picker = view.rootObject()->findChild<QObject *>(u"contentSourcePicker"_s);
        return picker && picker->property("opened").toBool();
    }, u"Sources overlay entry"_s);
    auto *content = picker->property("contentItem").value<QQuickItem *>();
    auto *toggle = visualItem(content, u"contentSourceEnabled_aniliberty"_s);
    auto *localRetry = visualItem(content, u"contentSourceRetry_aniliberty"_s);
    auto *globalRetry = picker->findChild<QQuickItem *>(u"contentSourceSettingsRetry"_s);
    auto *sourceError = visualItem(content, u"contentSourceFailureText_aniliberty"_s);
    auto *searchError = view.rootObject()->findChild<QQuickItem *>(u"contentError"_s);
    require(toggle && localRetry && globalRetry && sourceError && searchError
        && toggle->property("checked").toBool() && sourceError->isVisible() && !searchError->isVisible(),
        u"Source failure was not visible only inside the production Sources overlay"_s);
    const auto key = [&]
    {
        toggle->forceActiveFocus(Qt::TabFocusReason);
        QKeyEvent press(QEvent::KeyPress, Qt::Key_Space, Qt::NoModifier);
        QCoreApplication::sendEvent(&view, &press);
        QKeyEvent release(QEvent::KeyRelease, Qt::Key_Space, Qt::NoModifier);
        QCoreApplication::sendEvent(&view, &release);
    };
    key();
    saved(*uiController);
    require(!toggle->property("checked").toBool() && !localRetry->isEnabled(), u"Source switch did not disable its exact row/action"_s);
    exec(database, u"CREATE TRIGGER reject_source_ui BEFORE UPDATE ON source_settings BEGIN SELECT RAISE(ABORT, 'generated source UI fault'); END;"_s);
    key();
    saved(*uiController);
    require(toggle->property("checked").toBool() && globalRetry->isVisible() && globalRetry->isEnabled()
        && !localRetry->isEnabled() && !searchError->isVisible(), u"Failed source write exposed a no-op local retry or lost its chosen state"_s);
    const QString imageDirectory = qEnvironmentVariable("QBUTT_SOURCE_QA_IMAGES");
    if (!imageDirectory.isEmpty())
    {
        require(QDir().mkpath(imageDirectory), u"Source render directory failed"_s);
        for (const QSize &size : {QSize(1440, 900), QSize(1280, 720)})
        {
            view.resize(size);
            QEventLoop frame;
            QTimer::singleShot(200, &frame, &QEventLoop::quit);
            frame.exec();
            require(view.grabFramebuffer().save(QDir(imageDirectory).filePath(u"sources-%1x%2.png"_s.arg(size.width()).arg(size.height()))), u"Production source render failed"_s);
        }
    }
    const auto preserveContext = [&]
    {
        require((model(*uiController, "works")->rowCount() == workCount)
            && (uiController->property("selectedWorkId").toString() == selectedWork)
            && (uiController->property("selectedVariantId").toString() == selectedVariant)
            && !uiController->property("busy").toBool(), u"Source recovery replaced the existing search/selection"_s);
    };
    for (const bool returnKey : {false, true})
    {
        auto *close = picker->findChild<QQuickItem *>(u"contentSourcePickerClose"_s);
        require(close && QMetaObject::invokeMethod(close, "clicked"), u"Sources recovery overlay close failed"_s);
        waitFor([&] { return !view.rootObject()->findChild<QObject *>(u"contentSourcePicker"_s); }, u"Sources recovery overlay exit"_s);
        auto *query = view.rootObject()->findChild<QQuickItem *>(u"contentQuery"_s);
        require(query && query->setProperty("text", u"Frieren next"_s), u"Source recovery query field is missing"_s);
        if (returnKey)
        {
            query->forceActiveFocus(Qt::TabFocusReason);
            QKeyEvent press(QEvent::KeyPress, Qt::Key_Return, Qt::NoModifier);
            QCoreApplication::sendEvent(&view, &press);
            QKeyEvent release(QEvent::KeyRelease, Qt::Key_Return, Qt::NoModifier);
            QCoreApplication::sendEvent(&view, &release);
        }
        else
        {
            auto *search = view.rootObject()->findChild<QQuickItem *>(u"contentSearch"_s);
            require(search && QMetaObject::invokeMethod(search, "clicked"), u"Explicit source-blocked Search action failed"_s);
        }
        waitFor([&]
        {
            picker = view.rootObject()->findChild<QObject *>(u"contentSourcePicker"_s);
            return picker && picker->property("opened").toBool();
        }, u"Explicit blocked Search opened recovery"_s);
        require(query->property("text").toString() == u"Frieren next"_s, u"Source recovery changed the entered query"_s);
        preserveContext();
    }
    content = picker->property("contentItem").value<QQuickItem *>();
    toggle = visualItem(content, u"contentSourceEnabled_aniliberty"_s);
    localRetry = visualItem(content, u"contentSourceRetry_aniliberty"_s);
    globalRetry = picker->findChild<QQuickItem *>(u"contentSourceSettingsRetry"_s);
    require(toggle && localRetry && globalRetry, u"Reopened recovery controls are missing"_s);
    exec(database, u"DROP TRIGGER reject_source_ui"_s);
    require(QMetaObject::invokeMethod(globalRetry, "clicked"), u"Production global Sources retry failed"_s);
    saved(*uiController);
    require(state(*uiController) == Qbutt::Sources::SettingsState::Ready
        && toggle->property("checked").toBool() && !globalRetry->isVisible() && localRetry->isEnabled(), u"Production Sources retry did not confirm the exact chosen setting"_s);
    auto *close = picker->findChild<QQuickItem *>(u"contentSourcePickerClose"_s);
    require(close && QMetaObject::invokeMethod(close, "clicked"), u"Sources overlay close failed"_s);
    waitFor([&] { return !view.rootObject()->findChild<QObject *>(u"contentSourcePicker"_s); }, u"Sources overlay exit"_s);
    require(QMetaObject::invokeMethod(sourceButton, "clicked"), u"Sources overlay reopen failed"_s);
    waitFor([&]
    {
        auto *reopened = view.rootObject()->findChild<QObject *>(u"contentSourcePicker"_s);
        auto *choice = reopened ? visualItem(reopened->property("contentItem").value<QQuickItem *>(), u"contentSourceEnabled_aniliberty"_s) : nullptr;
        return reopened && reopened->property("opened").toBool() && choice && choice->property("checked").toBool();
    }, u"Sources overlay retained choice"_s);
    require(!searchError->isVisible(), u"Closing Sources leaked its source failure into search"_s);

    qInfo("PASS sources=typed persisted_choices=restart loading_search=gated sqlite_write_retry=exact reentrant_choice=latest corrupt_source=isolated late_success=retained actual_inspect_search_retry=cleared actual_missing_publication=recorded source_handoff=none production_overlay=close_reopen source_switch=keyboard source_SQL_retry=QML");
}
