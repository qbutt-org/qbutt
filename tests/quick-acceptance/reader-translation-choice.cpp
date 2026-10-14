// SPDX-License-Identifier: GPL-2.0-or-later
#include "reader-translation-choice.h"

#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QDebug>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QImage>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QQuickItem>
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

void exerciseReaderOpenRouterSettings(Content::ContentView *view, const QString &profileDir)
{
    auto *controller = view->findChild<Qbutt::ContentController *>();
    require(controller, u"Reader controller missing"_s);
    waitFor([&] { return controller->property("libraryState").value<Qbutt::Library::LibraryState>()
        == Qbutt::Library::LibraryState::Ready; }, u"OpenRouter library"_s);
    controller->importLocalBook(QUrl::fromLocalFile(profileDir + u"/authored.html"_s));
    waitFor([&] { return controller->property("readerOpen").toBool()
        && !controller->property("readerBusy").toBool(); }, u"Authored original"_s);
    auto *loader = view->rootObject()->findChild<QObject *>(u"contentReaderLoader"_s);
    auto *reader = loader ? loader->property("item").value<QQuickItem *>() : nullptr;
    require(reader, u"Production reader missing"_s);
    const auto item = [&](const QString &name)
    {
        auto *result = reader->findChild<QQuickItem *>(name);
        require(result, u"Missing OpenRouter control: "_s + name);
        return result;
    };
    const auto click = [&](QQuickItem *target)
    {
        const QPointF point = target->mapToScene(QPointF(target->width() / 2, target->height() / 2));
        const QPointF global = view->mapToGlobal(point.toPoint());
        QMouseEvent press(QEvent::MouseButtonPress, point, global, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(view, &press);
        QMouseEvent release(QEvent::MouseButtonRelease, point, global, Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
        QCoreApplication::sendEvent(view, &release);
    };
    const auto content = [&]
    {
        auto *blocks = qobject_cast<QAbstractItemModel *>(controller->property("readerTextBlocks").value<QObject *>());
        require(blocks, u"Native text model missing"_s);
        const int role = blocks->roleNames().key("plainText", -1);
        QStringList result;
        for (int row = 0; row < blocks->rowCount(); ++row)
            result.append(blocks->data(blocks->index(row, 0), role).toString());
        return result;
    };
    const auto capture = [&](const QString &name, const QSize &size)
    {
        view->resize(size);
        QEventLoop frame;
        QTimer::singleShot(200, &frame, &QEventLoop::quit);
        frame.exec();
        const QImage image = view->grabFramebuffer();
        require(image.size() == size && image.save(profileDir + u"/"_s + name), u"OpenRouter render failed"_s);
    };
    const auto pressKey = [&](const Qt::Key key)
    {
        QKeyEvent press(QEvent::KeyPress, key, Qt::NoModifier);
        QCoreApplication::sendEvent(view, &press);
        QKeyEvent release(QEvent::KeyRelease, key, Qt::NoModifier);
        QCoreApplication::sendEvent(view, &release);
    };
    const auto enterText = [&](QQuickItem *field, const QString &text)
    {
        click(field);
        QKeyEvent select(QEvent::KeyPress, Qt::Key_A, Qt::ControlModifier);
        QCoreApplication::sendEvent(view, &select);
        QKeyEvent input(QEvent::KeyPress, 0, Qt::NoModifier, text);
        QCoreApplication::sendEvent(view, &input);
        require(field->property("text").toString() == text, u"Text field did not accept input"_s);
    };
    const QString connectionName = u"reader-openrouter-settings"_s;
    auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, connectionName);
    const auto closeDatabase = qScopeGuard([&]
    {
        database.close();
        database = {};
        QSqlDatabase::removeDatabase(connectionName);
    });
    database.setDatabaseName(profileDir + u"/content-library.sqlite"_s);
    require(database.open(), database.lastError().text());
    const auto execute = [&](const QString &sql)
    {
        QSqlQuery query(database);
        require(query.exec(sql), query.lastError().text());
    };
    const QString failWrite = u"CREATE TRIGGER fail_service_write BEFORE UPDATE ON text_translation_service "
        u"BEGIN SELECT RAISE(ABORT, 'fixture'); END;"_s;
    const QStringList original = content();
    auto *popup = reader->findChild<QObject *>(u"readerTranslationPicker"_s);
    click(item(u"readerTranslationSettings"_s));
    waitFor([&] { return popup && popup->property("visible").toBool()
        && !controller->property("readerTranslationOptionsLoading").toBool(); }, u"OpenRouter options"_s);
    require(controller->property("readerTextTranslationService").toString() == u"openrouter"_s
        && controller->property("readerTranslationModelCount").toInt() == 0,
        u"OpenRouter settings depend on Codex discovery"_s);
    auto *model = item(u"readerOpenRouterModel"_s);
    auto *key = item(u"readerOpenRouterKey"_s);
    auto *apply = item(u"readerTranslationApply"_s);
    auto *error = item(u"readerTranslationSettingsError"_s);
    require(key->property("text").toString().isEmpty() && key->property("echoMode").toInt() == 2,
        u"Key field is not empty and masked"_s);
    capture(u"openrouter-empty-1440.png"_s, QSize(1440, 900));
    enterText(model, u"invalid"_s);
    click(apply);
    waitFor([&] { return controller->property("readerTranslationSettingsError").toInt() == 1; }, u"Model validation"_s);
    require(error->isVisible() && model->property("text").toString() == u"invalid"_s,
        u"Validation discarded the user's model or hid its error"_s);
    capture(u"openrouter-invalid-1280.png"_s, QSize(1280, 720));
    enterText(model, u"example/text-model"_s);
    enterText(key, u"generated-fixture-key"_s);
    execute(failWrite);
    click(apply);
    waitFor([&] { return !controller->property("readerTranslationOptionsLoading").toBool()
        && controller->property("readerTranslationSettingsError").toInt() == 3; }, u"Failed key save"_s);
    require(!controller->property("readerOpenRouterKeyConfigured").toBool()
        && key->property("text").toString() == u"generated-fixture-key"_s
        && model->property("text").toString() == u"example/text-model"_s
        && item(u"readerTranslationSaveError"_s)->isVisible(),
        u"Failed key save lost its retry input or reported success"_s);
    capture(u"openrouter-save-error-1280.png"_s, QSize(1280, 720));
    execute(u"DROP TRIGGER fail_service_write"_s);
    click(apply);
    waitFor([&] { return !controller->property("readerTranslationOptionsLoading").toBool()
        && controller->property("readerOpenRouterKeyConfigured").toBool(); }, u"Protected key save retry"_s);
    require(key->property("text").toString().isEmpty(), u"Saved key remained in UI"_s);
    capture(u"openrouter-saved-1440.png"_s, QSize(1440, 900));
    execute(failWrite);
    click(item(u"readerOpenRouterRemoveKey"_s));
    click(apply);
    waitFor([&] { return !controller->property("readerTranslationOptionsLoading").toBool()
        && controller->property("readerTranslationSettingsError").toInt() == 3; }, u"Failed key removal"_s);
    require(controller->property("readerOpenRouterKeyConfigured").toBool()
        && popup->property("removeKey").toBool()
        && item(u"readerTranslationSaveError"_s)->isVisible(),
        u"Failed removal lost its intent or reported the saved key removed"_s);
    execute(u"DROP TRIGGER fail_service_write"_s);
    click(apply);
    waitFor([&] { return !controller->property("readerTranslationOptionsLoading").toBool()
        && !controller->property("readerOpenRouterKeyConfigured").toBool(); }, u"Explicit key removal retry"_s);
    auto *service = item(u"readerTranslationService"_s);
    click(service);
    pressKey(Qt::Key_Home);
    pressKey(Qt::Key_Return);
    click(apply);
    waitFor([&] { return !controller->property("readerTranslationOptionsLoading").toBool()
        && controller->property("readerTextTranslationService").toString() == u"codex"_s; }, u"Explicit Codex service"_s);
    require(!model->isVisible() && item(u"readerTranslationModel"_s)->isVisible(),
        u"Codex service did not switch its model controls"_s);
    capture(u"codex-settings-1280.png"_s, QSize(1280, 720));
    click(service);
    pressKey(Qt::Key_End);
    pressKey(Qt::Key_Return);
    click(apply);
    waitFor([&] { return !controller->property("readerTranslationOptionsLoading").toBool()
        && controller->property("readerTextTranslationService").toString() == u"openrouter"_s; }, u"Explicit OpenRouter service"_s);
    require(model->isVisible() && !item(u"readerTranslationModel"_s)->isVisible()
        && !controller->property("readerTranslationOptionsError").toBool(),
        u"OpenRouter remained dependent on the Codex catalog"_s);
    click(item(u"readerTranslationClose"_s));
    waitFor([&] { return !popup->property("visible").toBool(); }, u"Options dismissal"_s);
    require(controller->setReaderTextTranslation(true), u"Exact offline translation rejected"_s);
    waitFor([&] { return !controller->property("readerTranslationBusy").toBool()
        && content().value(0).startsWith(u"Cached: "_s); }, u"Exact offline cache"_s);
    require(!controller->property("readerTranslationError").toBool(), u"Offline cached translation failed"_s);
    const QStringList cached = content();
    require(controller->setReaderTextTranslation(false) && content() == original, u"Original text changed"_s);
    require(controller->setReaderTextTranslation(true), u"Cache restoration rejected"_s);
    waitFor([&] { return content() == cached; }, u"Repeated exact cache"_s);
    require(controller->setReaderTextTranslation(false), u"Return to Original rejected"_s);
    controller->closeReader();
    controller->importLocalBook(QUrl::fromLocalFile(profileDir + u"/authored.html"_s));
    waitFor([&] { return controller->property("readerOpen").toBool()
        && !controller->property("readerBusy").toBool(); }, u"Reader reopen"_s);
    require(controller->setReaderTextTranslation(true), u"Reopened offline cache rejected"_s);
    waitFor([&] { return content() == cached; }, u"Reopened exact cache"_s);
    require(controller->setReaderTextTranslation(false) && content() == original, u"Reopened Original changed"_s);

    require(controller->setReaderTextTranslation(true), u"Delayed save cache rejected"_s);
    waitFor([&] { return content() == cached; }, u"Delayed save cached translation"_s);
    execute(u"BEGIN IMMEDIATE"_s);
    require(controller->configureReaderTextTranslation(u"openrouter"_s, u"example/text-model"_s,
        u"low"_s, u"generated-delayed-key"_s, false), u"Delayed key save rejected"_s);
    require(controller->setReaderTextTranslation(false), u"Original during key save rejected"_s);
    controller->closeReader();
    controller->importLocalBook(QUrl::fromLocalFile(profileDir + u"/authored.html"_s));
    bool restoredAfterReopen = false;
    QObject observation;
    QObject::connect(controller, &Qbutt::ContentController::readerChanged, &observation, [&]
    {
        restoredAfterReopen |= controller->property("readerTranslationRequested").toBool();
    }, Qt::DirectConnection);
    execute(u"COMMIT"_s);
    waitFor([&] { return controller->property("readerOpen").toBool()
        && !controller->property("readerBusy").toBool()
        && !controller->property("readerTranslationOptionsLoading").toBool()
        && controller->property("readerOpenRouterKeyConfigured").toBool(); }, u"Delayed key save acknowledgement"_s);
    require(!restoredAfterReopen && !controller->property("readerTranslationRequested").toBool()
        && content() == original, u"Late key save restored translation in another reader session"_s);
    require(controller->configureReaderTextTranslation(u"openrouter"_s, u"example/text-model"_s,
        u"low"_s, u""_s, true), u"Delayed fixture key removal rejected"_s);
    waitFor([&] { return !controller->property("readerTranslationOptionsLoading").toBool()
        && !controller->property("readerOpenRouterKeyConfigured").toBool(); }, u"Delayed fixture key removal"_s);
    qInfo().noquote() << u"PASS reader_openrouter explicit_service model_error masked_key native_dpapi "
        u"save_failure_retry removal_failure_retry delayed_save_original offline_cache original reopen no_inference"_s;
}
