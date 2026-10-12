// SPDX-License-Identifier: GPL-2.0-or-later
#include "reader-settings.h"

#include <exception>
#include <functional>
#include <memory>
#include <stdexcept>

#include <QApplication>
#include <QDebug>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QImageReader>
#include <QProcess>
#include <QScopeGuard>
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QTemporaryDir>
#include <QThreadPool>
#include <QTimer>
#include <QUuid>
#include <QUrl>

#ifdef Q_OS_WIN
#include <Windows.h>
#endif

#include "qbutt_quick/src/controller.cxxqt.h"

using namespace Qt::Literals::StringLiterals;
using Fit = Qbutt::Reader::ReaderFit;
using SettingsState = Qbutt::Reader::ReaderSettingsState;
using LibraryState = Qbutt::Library::LibraryState;
using ErrorKind = Qbutt::Library::LibraryErrorKind;

namespace
{
    void require(const bool condition, const QString &message)
    {
        if (!condition)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const std::function<bool()> &ready, const QString &operation)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < 100'000))
        {
            QEventLoop loop;
            QTimer::singleShot(20, &loop, &QEventLoop::quit);
            loop.exec();
        }
        require(ready(), operation + u" timed out"_s);
    }

    SettingsState settingsState(const Qbutt::ContentController &controller)
    {
        return controller.property("readerSettingsState").value<SettingsState>();
    }

    Fit appliedFit(const Qbutt::ContentController &controller)
    {
        return controller.property("readerFit").value<Fit>();
    }

    void execute(QSqlDatabase &database, const QString &sql)
    {
        QSqlQuery query(database);
        require(query.exec(sql), query.lastError().text());
    }

    int scalar(QSqlDatabase &database, const QString &sql)
    {
        QSqlQuery query(database);
        require(query.exec(sql) && query.next(), query.lastError().text());
        const int value = query.value(0).toInt();
        require(!query.next(), u"Scalar fixture returned more than one row"_s);
        return value;
    }

    void settle(Qbutt::ContentController &controller)
    {
        waitFor([&] { return !controller.property("libraryPending").toBool(); }, u"Settings FIFO drain"_s);
    }

    void initialize(Qbutt::ContentController &controller, const QString &profile)
    {
        controller.initLibrary(profile);
        waitFor([&]
        {
            const auto state = controller.property("libraryState").value<LibraryState>();
            return (state == LibraryState::Ready) || (state == LibraryState::Failed);
        }, u"Settings profile open"_s);
        require(controller.property("libraryState").value<LibraryState>() == LibraryState::Ready,
            controller.property("libraryErrorText").toString());
        settle(controller);
    }

    class Fixture
    {
    public:
        Fixture()
        {
            require(profile.isValid(), u"Reader settings profile creation failed"_s);
            controller = std::make_unique<Qbutt::ContentController>();
            initialize(*controller, profile.path());
            database = QSqlDatabase::addDatabase(u"QSQLITE"_s, connection);
            database.setDatabaseName(profile.filePath(u"content-library.sqlite"_s));
            require(database.open(), database.lastError().text());
        }

        ~Fixture()
        {
            database.close();
            database = QSqlDatabase();
            QSqlDatabase::removeDatabase(connection);
            controller.reset();
            if (!QThreadPool::globalInstance()->waitForDone(100'000))
                qFatal("Reader settings owner did not drain before profile destruction");
        }

        void retireController()
        {
            controller.reset();
            require(QThreadPool::globalInstance()->waitForDone(100'000), u"Settings owner retirement failed"_s);
            controller = std::make_unique<Qbutt::ContentController>();
        }

        QTemporaryDir profile;
        QString connection = QUuid::createUuid().toString();
        QSqlDatabase database;
        std::unique_ptr<Qbutt::ContentController> controller;
    };

    void expectSaved(Fixture &fixture, const Fit fit)
    {
        settle(*fixture.controller);
        require((settingsState(*fixture.controller) == SettingsState::Ready)
            && fixture.controller->property("readerSettingsErrorText").toString().isEmpty()
            && (appliedFit(*fixture.controller) == fit)
            && (scalar(fixture.database, u"SELECT fit FROM reader_settings WHERE id = 1"_s) == (fit == Fit::Width ? 1 : 0)),
            u"Reader settings did not confirm the actual committed fit"_s);
    }

    void exerciseLateLoadAndWrites()
    {
        Fixture fixture;
        auto &controller = *fixture.controller;
        require((appliedFit(controller) == Fit::Height) && (settingsState(controller) == SettingsState::Ready)
            && (scalar(fixture.database, u"SELECT count(*) FROM library"_s) == 0), u"Fresh profile defaults created history"_s);
        require(!controller.setReaderFit(static_cast<Fit>(42)) && (appliedFit(controller) == Fit::Height),
            u"Invalid fit changed the typed settings"_s);
        require(controller.setReaderFit(Fit::Width) && (settingsState(controller) == SettingsState::Saving),
            u"Accepted fit did not report Saving"_s);
        expectSaved(fixture, Fit::Width);

        fixture.retireController();
        bool lateIntentAccepted = false;
        QObject observer;
        QObject::connect(fixture.controller.get(), &Qbutt::ContentController::libraryStateChanged, &observer, [&]
        {
            if (fixture.controller->property("libraryState").value<LibraryState>() == LibraryState::Loading)
                lateIntentAccepted = fixture.controller->setReaderFit(Fit::Height);
        }, Qt::DirectConnection);
        initialize(*fixture.controller, fixture.profile.path());
        require(lateIntentAccepted, u"Loading did not accept the explicit default fit"_s);
        expectSaved(fixture, Fit::Height);

        require(fixture.controller->setReaderFit(Fit::Width), u"Could not prepare the ABA baseline"_s);
        expectSaved(fixture, Fit::Width);
        execute(fixture.database, u"CREATE TRIGGER reject_middle_fit BEFORE UPDATE ON reader_settings "
            "WHEN NEW.fit = 1 BEGIN SELECT RAISE(ABORT, 'generated middle fit fault'); END"_s);
        execute(fixture.database, u"BEGIN IMMEDIATE"_s);
        require(fixture.controller->setReaderFit(Fit::Height) && fixture.controller->setReaderFit(Fit::Width)
            && fixture.controller->setReaderFit(Fit::Height), u"Rapid ABA fits were not accepted in FIFO order"_s);
        require((settingsState(*fixture.controller) == SettingsState::Saving)
            && (scalar(fixture.database, u"SELECT fit FROM reader_settings"_s) == 1), u"Blocked write claimed persistence"_s);
        int latestAcknowledgements = 0;
        bool staleFit = false;
        const auto acknowledgement = QObject::connect(fixture.controller.get(), &Qbutt::ContentController::readerSettingsChanged,
            &observer, [&]
        {
            ++latestAcknowledgements;
            staleFit = staleFit || (appliedFit(*fixture.controller) != Fit::Height)
                || (settingsState(*fixture.controller) == SettingsState::Failed);
        }, Qt::DirectConnection);
        execute(fixture.database, u"COMMIT"_s);
        expectSaved(fixture, Fit::Height);
        QObject::disconnect(acknowledgement);
        require(!staleFit && (latestAcknowledgements == 1), u"An older ABA acknowledgement published newer settings status"_s);
        execute(fixture.database, u"DROP TRIGGER reject_middle_fit"_s);

        bool nestedIntentAccepted = false;
        const auto nested = QObject::connect(fixture.controller.get(), &Qbutt::ContentController::readerSettingsChanged,
            &observer, [&]
        {
            if (!nestedIntentAccepted && (settingsState(*fixture.controller) == SettingsState::Saving))
            {
                nestedIntentAccepted = true;
                fixture.controller->setReaderFit(Fit::Height);
            }
        }, Qt::DirectConnection);
        require(fixture.controller->setReaderFit(Fit::Width), u"Reentrant fit was rejected"_s);
        expectSaved(fixture, Fit::Height);
        QObject::disconnect(nested);
        require(nestedIntentAccepted, u"Settings notification did not exercise a reentrant latest intent"_s);

        execute(fixture.database, u"CREATE TRIGGER reject_fit BEFORE UPDATE ON reader_settings "
            "BEGIN SELECT RAISE(ABORT, 'generated fit write fault'); END"_s);
        require(fixture.controller->setReaderFit(Fit::Width), u"SQL abort fit was not accepted"_s);
        settle(*fixture.controller);
        require((settingsState(*fixture.controller) == SettingsState::Failed)
            && (appliedFit(*fixture.controller) == Fit::Width)
            && (scalar(fixture.database, u"SELECT fit FROM reader_settings"_s) == 0)
            && (fixture.controller->property("readerSettingsErrorText").toString()
                == QCoreApplication::translate("ContentController", "Could not save reading settings.")),
            u"Aborted settings write claimed persistence or lost its exact intent"_s);
        require(fixture.controller->refreshHistory(), u"Unrelated history refresh was rejected"_s);
        settle(*fixture.controller);
        require(settingsState(*fixture.controller) == SettingsState::Failed, u"Unrelated History success hid failed fit"_s);
        execute(fixture.database, u"DROP TRIGGER reject_fit"_s);
        require(fixture.controller->retryReaderSettings(), u"Exact fit retry was rejected"_s);
        expectSaved(fixture, Fit::Width);
        require(!fixture.controller->retryReaderSettings(), u"Committed settings remained retryable"_s);

        execute(fixture.database, u"BEGIN IMMEDIATE"_s);
        require(fixture.controller->setReaderFit(Fit::Height), u"Queue fault did not block its writer"_s);
        bool rejected = false;
        Fit rejectedFit = Fit::Height;
        for (int request = 0; (request < 80) && !rejected; ++request)
        {
            rejectedFit = request % 2 ? Fit::Height : Fit::Width;
            rejected = !fixture.controller->setReaderFit(rejectedFit);
        }
        require(rejected && (settingsState(*fixture.controller) == SettingsState::Failed), u"Settings FIFO did not reject a full queue"_s);
        execute(fixture.database, u"COMMIT"_s);
        settle(*fixture.controller);
        require((settingsState(*fixture.controller) == SettingsState::Failed) && (appliedFit(*fixture.controller) == rejectedFit)
            && (fixture.controller->property("libraryErrorKind").value<ErrorKind>() == ErrorKind::None),
            u"Older completions hid the rejected exact fit or created another error owner"_s);
        require(fixture.controller->retryReaderSettings(), u"Rejected fit could not retry after FIFO drain"_s);
        expectSaved(fixture, rejectedFit);
    }

    void exerciseStartupFailure(const bool corruptSettings)
    {
        Fixture fixture;
        fixture.retireController();
        if (corruptSettings)
        {
            execute(fixture.database, u"PRAGMA ignore_check_constraints = ON"_s);
            execute(fixture.database, u"UPDATE reader_settings SET fit = 99"_s);
            execute(fixture.database, u"PRAGMA ignore_check_constraints = OFF"_s);
        }
        else
            execute(fixture.database, u"BEGIN EXCLUSIVE"_s);
        bool accepted = false;
        int loadingCount = 0;
        QObject observer;
        QObject::connect(fixture.controller.get(), &Qbutt::ContentController::libraryStateChanged, &observer, [&]
        {
            if (fixture.controller->property("libraryState").value<LibraryState>() == LibraryState::Loading)
            {
                ++loadingCount;
                if (loadingCount == 1)
                    accepted = fixture.controller->setReaderFit(Fit::Width);
                else if (!corruptSettings)
                    accepted = fixture.controller->setReaderFit(Fit::Height);
            }
        }, Qt::DirectConnection);
        fixture.controller->initLibrary(fixture.profile.path());
        waitFor([&] { return fixture.controller->property("libraryState").value<LibraryState>() == LibraryState::Failed; },
            u"Startup settings failure"_s);
        require(accepted && (settingsState(*fixture.controller) == SettingsState::Failed)
            && !fixture.controller->property("libraryPending").toBool() && (appliedFit(*fixture.controller) == Fit::Width),
            u"Failed owner left an accepted fit Saving or substituted its value"_s);
        if (corruptSettings)
            execute(fixture.database, u"UPDATE reader_settings SET fit = 0"_s);
        else
            execute(fixture.database, u"COMMIT"_s);
        require(fixture.controller->retryReaderSettings(), u"Settings retry did not reopen the existing owner"_s);
        waitFor([&] { return fixture.controller->property("libraryState").value<LibraryState>() == LibraryState::Ready; },
            u"Same settings owner recovery"_s);
        require((loadingCount == 2) && accepted, u"Same worker retry did not accept the newer Loading intent"_s);
        expectSaved(fixture, corruptSettings ? Fit::Width : Fit::Height);
    }

    void exerciseOtherOutcomes()
    {
        Fixture fixture;
        auto &controller = *fixture.controller;
        execute(fixture.database, u"CREATE TRIGGER reject_fit BEFORE UPDATE ON reader_settings "
            "BEGIN SELECT RAISE(ABORT, 'generated independent fit fault'); END"_s);
        require(controller.setReaderFit(Fit::Width), u"Independent fit fault was rejected"_s);
        settle(controller);
        require(settingsState(controller) == SettingsState::Failed, u"Independent fit fault did not fail"_s);
        controller.search(u"Frieren"_s);
        waitFor([&] { return !controller.property("busy").toBool(); }, u"Settings integration live search"_s);
        controller.selectWork(u"mangadex:b0b721ff-c388-4486-aa0f-c2b0bb321512"_s);
        waitFor([&] { return !controller.property("inspecting").toBool(); }, u"Settings integration live inspect"_s);
        require(controller.property("errorText").toString().isEmpty(), controller.property("errorText").toString());
        controller.setSelectedFavorite(true);
        settle(controller);
        require(controller.property("selectedFavorite").toBool() && (settingsState(controller) == SettingsState::Failed),
            u"Favorite success cleared failed settings"_s);
        controller.selectVariant(u"mangadex:b0b721ff-c388-4486-aa0f-c2b0bb321512:translation:ru:9e383226-2af3-4af0-a69d-3d3b630212b3"_s);
        settle(controller);
        controller.openReader();
        waitFor([&] { return !controller.property("readerBusy").toBool(); }, u"Settings integration actual reader acquisition"_s);
        require(controller.property("readerErrorText").toString().isEmpty(), controller.property("readerErrorText").toString());
        const auto url = controller.property("readerPageUrl").toUrl();
        {
            QImageReader decoder(url.toLocalFile());
            require(url.isLocalFile() && !decoder.read().isNull(), u"Settings scenario did not decode an actual local page"_s);
        }
        controller.readerPageShown(controller.property("readerPageIndex").toInt(), url);
        settle(controller);
        require(controller.property("readerProgressErrorText").toString().isEmpty()
            && (settingsState(controller) == SettingsState::Failed)
            && (scalar(fixture.database, u"SELECT count(*) FROM library WHERE json_extract(entry, '$.last_opened') IS NOT NULL"_s) == 1),
            u"Successful confirmed reading hid fit failure or failed to record history"_s);
        execute(fixture.database, u"CREATE TRIGGER reject_bookmark BEFORE UPDATE ON library "
            "BEGIN SELECT RAISE(ABORT, 'generated unrelated bookmark fault'); END"_s);
        controller.setSelectedFavorite(false);
        settle(controller);
        require((controller.property("libraryErrorKind").value<ErrorKind>() == ErrorKind::Writing)
            && (settingsState(controller) == SettingsState::Failed), u"Two independent write failures were conflated"_s);
        execute(fixture.database, u"DROP TRIGGER reject_fit"_s);
        require(controller.retryReaderSettings(), u"Independent settings retry was rejected"_s);
        expectSaved(fixture, Fit::Width);
        require((controller.property("libraryErrorKind").value<ErrorKind>() == ErrorKind::Writing)
            && controller.property("selectedFavorite").toBool(), u"Settings success hid another failed write"_s);
        execute(fixture.database, u"DROP TRIGGER reject_bookmark"_s);
        controller.setSelectedFavorite(false);
        settle(controller);
        require(!controller.property("selectedFavorite").toBool() && (settingsState(controller) == SettingsState::Ready),
            u"Explicit bookmark retry damaged committed settings"_s);
        controller.closeReader();
        fixture.retireController();
        initialize(*fixture.controller, fixture.profile.path());
        expectSaved(fixture, Fit::Width);
        require(scalar(fixture.database, u"SELECT count(*) FROM library WHERE json_extract(entry, '$.last_opened') IS NOT NULL"_s) == 1,
            u"Settings restart removed confirmed history"_s);
    }

    void exerciseProcessPhase(const QString &profile, const QString &phase)
    {
        require(!profile.isEmpty(), u"Reader settings process phase requires an isolated profile"_s);
        auto controller = std::make_unique<Qbutt::ContentController>();
        initialize(*controller, profile);
        if (phase == u"reopen"_s)
        {
            require((appliedFit(*controller) == Fit::Width)
                && (controller->property("readerMode").value<Qbutt::Reader::ReaderMode>() == Qbutt::Reader::ReaderMode::Continuous)
                && (settingsState(*controller) == SettingsState::Ready),
                u"Fresh process did not restore committed Continuous/Width"_s);
            controller.reset();
        }
        else
        {
            require(phase == u"commit"_s, u"Unknown reader settings process phase"_s);
            const auto connection = QUuid::createUuid().toString();
            auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, connection);
            const auto close = qScopeGuard([&]
            {
                database.close();
                database = QSqlDatabase();
                QSqlDatabase::removeDatabase(connection);
            });
            database.setDatabaseName(profile + u"/content-library.sqlite"_s);
            require(database.open(), database.lastError().text());
            execute(database, u"BEGIN IMMEDIATE"_s);
            require(controller->setReaderMode(Qbutt::Reader::ReaderMode::Continuous)
                && controller->setReaderFit(Fit::Width) && (settingsState(*controller) == SettingsState::Saving),
                u"Graceful Exit did not accept its blocked settings write"_s);
            controller.reset();
            execute(database, u"COMMIT"_s);
        }
        require(QThreadPool::globalInstance()->waitForDone(100'000), u"Graceful settings process did not drain"_s);
        qInfo().noquote() << u"PASS settings_process phase="_s + phase;
    }

    void exerciseProcessRestart()
    {
        QTemporaryDir profile;
        require(profile.isValid(), u"Settings process profile creation failed"_s);
        QFile unrelated(profile.filePath(u"downloaded-payload.txt"_s));
        require(unrelated.open(QIODevice::WriteOnly) && (unrelated.write("generated unrelated payload") == 27),
            u"Could not create the settings payload sentinel"_s);
        unrelated.close();
        for (const auto &phase : {u"commit"_s, u"reopen"_s})
        {
            QProcess child;
#ifdef Q_OS_WIN
            child.setCreateProcessArgumentsModifier([](QProcess::CreateProcessArguments *arguments)
            {
                arguments->flags |= CREATE_NO_WINDOW;
            });
#endif
            child.setProcessChannelMode(QProcess::MergedChannels);
            child.start(QCoreApplication::applicationFilePath(), {u"--reader-settings"_s, u"--reader-settings-phase"_s,
                phase, u"--profile"_s, profile.path()});
            if (!child.waitForFinished(100'000))
            {
                child.kill();
                child.waitForFinished();
                throw std::runtime_error("Settings restart process timed out");
            }
            const auto output = child.readAll();
            require((child.exitStatus() == QProcess::NormalExit) && (child.exitCode() == 0),
                u"Settings process %1 exited %2: %3"_s.arg(phase).arg(child.exitCode()).arg(QString::fromUtf8(output)));
            qInfo().noquote() << u"settings_process phase=%1 exit=0"_s.arg(phase);
        }
        const auto connection = QUuid::createUuid().toString();
        auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, connection);
        const auto close = qScopeGuard([&]
        {
            database.close();
            database = QSqlDatabase();
            QSqlDatabase::removeDatabase(connection);
        });
        database.setDatabaseName(profile.filePath(u"content-library.sqlite"_s));
        require(database.open(), database.lastError().text());
        require((scalar(database, u"SELECT fit FROM reader_settings"_s) == 1)
            && (scalar(database, u"SELECT mode FROM reader_settings"_s) == 1)
            && (scalar(database, u"SELECT count(*) FROM library"_s) == 0),
            u"Settings process did not persist exact Width or invented history"_s);
        require(unrelated.open(QIODevice::ReadOnly) && (unrelated.readAll() == "generated unrelated payload"),
            u"Settings process changed unrelated payload"_s);
    }
}

void exerciseReaderSettings(const QString &profile, const QString &phase)
{
    if (!phase.isEmpty())
    {
        exerciseProcessPhase(profile, phase);
        return;
    }
    exerciseLateLoadAndWrites();
    exerciseStartupFailure(false);
    exerciseStartupFailure(true);
    exerciseProcessRestart();
    exerciseOtherOutcomes();
    qInfo().noquote() << u"PASS reader_settings default_height=ok explicit_loading=ok invalid_fit=ok committed_state=ok late_load=ok rapid_aba=ok reentrant_intent=ok sql_abort_retry=ok queue_rejection_retry=ok startup_failure_retry=ok strict_settings_retry=ok independent_history=ok independent_progress=ok independent_bookmark=ok controller_restart=ok process_restart=ok graceful_accepted_write=ok payload_preserved=ok"_s;
}
