// SPDX-License-Identifier: GPL-2.0-or-later
#include "reader-translation-choice.h"

#include <functional>
#include <stdexcept>

#include <QDebug>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QScopeGuard>
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QTimer>
#include <QUrl>

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

    void waitFor(const std::function<bool()> &condition, const QString &operation)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!condition() && (elapsed.elapsed() < 100'000))
        {
            QEventLoop loop;
            QTimer::singleShot(20, &loop, &QEventLoop::quit);
            loop.exec();
        }
        require(condition(), operation + u" timed out"_s);
    }
}

void exerciseReaderTranslationChoiceRace(Content::ContentView *view, const QString &profileDir)
{
    auto *controller = view->findChild<Qbutt::ContentController *>();
    require(controller, u"Translation choice controller is missing"_s);
    waitFor(
        [&]
        {
            return controller->property("libraryState").value<Qbutt::Library::LibraryState>() ==
                Qbutt::Library::LibraryState::Ready;
        },
        u"Translation choice library"_s);
    controller->search(u"Frieren"_s);
    waitFor([&] { return !controller->property("busy").toBool(); }, u"Translation choice search"_s);
    const QString publication = u"mangadex:b0b721ff-c388-4486-aa0f-c2b0bb321512"_s;
    controller->selectWork(publication);
    waitFor([&] { return !controller->property("inspecting").toBool(); }, u"Translation choice inspection"_s);
    require(controller->property("errorText").toString().isEmpty(), u"Translation choice inspection failed"_s);
    const QString variant = publication + u":translation:ru:9e383226-2af3-4af0-a69d-3d3b630212b3"_s;
    controller->selectVariant(variant);
    require(controller->property("selectedVariantId").toString() == variant,
            u"Translation choice manga variant is missing"_s);
    controller->openReader();
    waitFor([&] { return !controller->property("readerBusy").toBool(); }, u"Translation choice reader"_s);
    require(controller->property("readerErrorText").toString().isEmpty() &&
                controller->property("readerPageUrl").toUrl().isLocalFile(),
            u"Translation choice page was not acquired"_s);

    const QString connection = u"reader-translation-choice"_s;
    auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, connection);
    const auto closeDatabase = qScopeGuard(
        [&]
        {
            database.close();
            database = {};
            QSqlDatabase::removeDatabase(connection);
        });
    database.setDatabaseName(profileDir + u"/content-library.sqlite"_s);
    require(database.open(), database.lastError().text());
    const auto execute = [&](const QString &sql)
    {
        QSqlQuery query(database);
        require(query.exec(sql), query.lastError().text());
    };
    const auto storedExplicit = [&]
    {
        QSqlQuery query(database);
        require(query.exec(u"SELECT explicit FROM translation_selection WHERE target_language = 'ru'"_s)
                && query.next(), query.lastError().text());
        return query.value(0).toInt();
    };
    const auto selectExplicitModel = [&]
    {
        execute(u"INSERT INTO translation_selection (target_language, model, "
                u"effort, explicit) "
                u"VALUES ('ru', 'gpt-6.1-sol', 'low', 1) "
                u"ON CONFLICT (target_language) DO UPDATE SET explicit = 1"_s);
        require(controller->loadReaderTranslationOptions(), u"Translation options did not reload"_s);
        waitFor([&] { return !controller->property("readerTranslationOptionsLoading").toBool(); },
                u"Saved translation choice load"_s);
        require(controller->property("readerTranslationModel").toString() == u"gpt-6.1-sol"_s,
                u"Saved explicit translation choice was not loaded"_s);
    };

    selectExplicitModel();
    // Hold the SQLite write until the user has changed the reader state.
    execute(u"BEGIN IMMEDIATE"_s);
    require(controller->setReaderMangaTranslation(true) &&
                controller->property("readerTranslationRequested").toBool() &&
                controller->setReaderTranslationModel(u""_s),
            u"Could not start translated reader choice save"_s);
    require(controller->setReaderMangaTranslation(false), u"Original reading choice was rejected"_s);
    bool restoredAfterOriginal = false;
    QObject observation;
    QObject::connect(
        controller, &Qbutt::ContentController::readerChanged, &observation,
        [&] { restoredAfterOriginal |= controller->property("readerTranslationRequested").toBool(); },
        Qt::DirectConnection);
    execute(u"COMMIT"_s);
    waitFor([&] { return !controller->property("readerTranslationOptionsLoading").toBool(); },
            u"Blocked translation choice save"_s);
    require(!restoredAfterOriginal && !controller->property("readerTranslationRequested").toBool() &&
                controller->property("readerTranslationModel").toString().isEmpty(),
            u"Late choice save overrode Original or lost its acknowledgement"_s);

    selectExplicitModel();
    execute(u"BEGIN IMMEDIATE"_s);
    require(controller->setReaderMangaTranslation(true) && controller->setReaderTranslationModel(u""_s),
            u"Could not start next-session translation choice save"_s);
    require(controller->loadReaderTranslationOptions() &&
                (controller->property("readerTranslationModel").toString() == u"gpt-6.1-sol"_s),
            u"Reopening the choice panel discarded its pending selection"_s);
    controller->closeReader();
    controller->openReader();
    require(controller->property("readerOpen").toBool(), u"Reader did not reopen during the blocked choice save"_s);
    bool restoredInNewSession = false;
    QObject::connect(
        controller, &Qbutt::ContentController::readerChanged, &observation, [&]
        { restoredInNewSession |= controller->property("readerTranslationRequested").toBool(); }, Qt::DirectConnection);
    execute(u"COMMIT"_s);
    waitFor([&] { return !controller->property("readerTranslationOptionsLoading").toBool(); },
            u"Previous-session translation choice save"_s);
    require(!restoredInNewSession && !controller->property("readerTranslationRequested").toBool(),
            u"Previous reader session enabled translation in the reopened reader"_s);

    selectExplicitModel();
    execute(u"BEGIN IMMEDIATE"_s);
    require(controller->setReaderMangaTranslation(true) && controller->setReaderTranslationModel(u""_s),
            u"Could not start current-session translation choice save"_s);
    bool restoredInSameSession = false;
    QObject::connect(
        controller, &Qbutt::ContentController::readerChanged, &observation, [&]
        { restoredInSameSession |= controller->property("readerTranslationRequested").toBool(); }, Qt::DirectConnection);
    execute(u"COMMIT"_s);
    waitFor([&] { return !controller->property("readerTranslationOptionsLoading").toBool(); },
            u"Current-session translation choice save"_s);
    require(restoredInSameSession, u"Current reader translation was not restored after its choice saved"_s);
    require(controller->setReaderMangaTranslation(false), u"Current reader did not return to Original"_s);

    selectExplicitModel();
    execute(u"BEGIN IMMEDIATE"_s);
    require(controller->setReaderTranslationModel(u""_s) && controller->setReaderTranslationLanguage(u"en"_s) &&
                controller->loadReaderTranslationOptions(),
            u"Changing target language lost the pending translation choice"_s);
    require(controller->loadReaderTranslationOptions(), u"Reopening the new language panel was rejected"_s);
    require(controller->property("readerTranslationLanguage").toString() == u"en"_s,
            u"New translation target was not applied"_s);
    execute(u"COMMIT"_s);
    waitFor(
        [&]
        {
            return !controller->property("readerTranslationOptionsLoading").toBool() &&
                storedExplicit() == 0 &&
                (controller->property("readerSettingsState").value<Qbutt::Reader::ReaderSettingsState>() ==
                 Qbutt::Reader::ReaderSettingsState::Ready);
        },
        u"Translation target and choice saves"_s);
    require(controller->property("readerTranslationLanguage").toString() == u"en"_s &&
                controller->property("readerTranslationModel").toString().isEmpty(),
            u"Old language choice completion changed the new panel state"_s);
    QSqlQuery savedLanguage(database);
    require(savedLanguage.exec(u"SELECT translation_language FROM reader_settings WHERE id = 1"_s)
            && savedLanguage.next() && savedLanguage.value(0).toString() == u"en"_s,
        u"New translation target was not persisted"_s);
    controller->closeReader();
    qInfo().noquote() << u"PASS reader_translation_choice delayed_original=ok session=ok same_session=ok reopen=ok language=ok"_s;
}
