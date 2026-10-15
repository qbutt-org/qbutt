// SPDX-License-Identifier: GPL-2.0-or-later
#include "reader-text-ahead.h"

#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QDebug>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QImage>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QScopeGuard>
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QTimer>
#include <QUrl>
#include <qt_windows.h>

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
        QEventLoop loop;
        QTimer timer;
        timer.setInterval(15);
        QObject::connect(&timer, &QTimer::timeout, &loop, [&]
        {
            if (condition() || elapsed.elapsed() >= 20'000)
                loop.quit();
        });
        if (!condition())
        {
            timer.start();
            loop.exec();
        }
        require(condition(), operation + u" timed out"_s);
    }

    void settle()
    {
        QEventLoop loop;
        QTimer::singleShot(200, &loop, &QEventLoop::quit);
        loop.exec();
    }

    QString content(Qbutt::ContentController *controller)
    {
        auto *model = qobject_cast<QAbstractItemModel *>(controller->property("readerTextBlocks").value<QObject *>());
        require(model, u"Text model missing"_s);
        const int role = model->roleNames().key("plainText", -1);
        QStringList blocks;
        for (int row = 0; row < model->rowCount(); ++row)
            blocks.append(model->data(model->index(row, 0), role).toString());
        return blocks.join(u'\n');
    }

    QList<QJsonObject> turns(const QDir &native)
    {
        QList<QJsonObject> result;
        for (const QString &name : native.entryList({u"turn-*.json"_s}, QDir::Files, QDir::Name))
        {
            QFile file(native.filePath(name));
            require(file.open(QIODevice::ReadOnly), u"Peer trace unavailable"_s);
            const auto turn = QJsonDocument::fromJson(file.readAll()).object();
            if (!turn.isEmpty())
                result.append(turn);
        }
        return result;
    }

    void hold(const QDir &native, const QString &chapter, const bool enabled)
    {
        QFile gate(native.filePath(u"hold-"_s + chapter));
        if (enabled)
            require(gate.open(QIODevice::WriteOnly), u"Peer gate unavailable"_s);
        else
            require(gate.remove(), u"Peer gate could not be released"_s);
    }

    bool exited(const QJsonObject &turn)
    {
        const DWORD pid = turn[u"pid"_s].toInteger();
        HANDLE process = OpenProcess(SYNCHRONIZE, FALSE, pid);
        if (!process)
            return GetLastError() == ERROR_INVALID_PARAMETER;
        const bool complete = WaitForSingleObject(process, 0) == WAIT_OBJECT_0;
        CloseHandle(process);
        return complete;
    }

    QVariant scalar(QSqlDatabase &database, const QString &sql)
    {
        QSqlQuery query(database);
        require(query.exec(sql) && query.next(), query.lastError().text());
        return query.value(0);
    }
}

void exerciseReaderTextAhead(Content::ContentView *view, const QString &profileDir)
{
    auto *controller = view->findChild<Qbutt::ContentController *>();
    require(controller, u"Reader controller missing"_s);
    const QString nativePath = qEnvironmentVariable("QBUTT_TEXT_PEER_DIRECTORY");
    require(!nativePath.isEmpty() && QDir::isAbsolutePath(nativePath), u"Controlled Codex peer required"_s);
    const QDir native(nativePath);
    const QByteArray programFiles = qgetenv("ProgramFiles");
    const QByteArray localAppData = qgetenv("LOCALAPPDATA");
    const QByteArray codexHome = qgetenv("CODEX_HOME");
    const QByteArray fixtureRoot = qgetenv("QBUTT_TEXT_PEER_ROOT");
    require(!fixtureRoot.isEmpty(), u"Controlled Codex discovery root required"_s);
    const auto restoreEnvironment = qScopeGuard([&]
    {
        qputenv("ProgramFiles", programFiles);
        qputenv("LOCALAPPDATA", localAppData);
        qputenv("CODEX_HOME", codexHome);
    });
    qputenv("ProgramFiles", fixtureRoot);
    qputenv("LOCALAPPDATA", fixtureRoot);
    qputenv("CODEX_HOME", fixtureRoot);
    require(turns(native).isEmpty(), u"Peer traces must start empty"_s);
    waitFor([&] { return controller->property("libraryState").value<Qbutt::Library::LibraryState>()
        == Qbutt::Library::LibraryState::Ready; }, u"Library"_s);

    const auto import = [&](const QString &title)
    {
        const QString path = profileDir + u"/"_s + title + u".fb2"_s;
        QByteArray book = "<?xml version=\"1.0\" encoding=\"utf-8\"?>"
            "<FictionBook xmlns=\"http://www.gribuser.ru/xml/fictionbook/2.0\">"
            "<description><title-info><genre>prose</genre><book-title>" + title.toUtf8()
            + "</book-title><lang>en</lang></title-info></description><body>";
        for (int chapter = 1; chapter <= 4; ++chapter)
        {
            book += "<section><title><p>Chapter " + QByteArray::number(chapter) + "</p></title><p>";
            book += chapter == 1 ? "Ren is female. Ren kept the blue key." : "Ren carried the blue key.";
            book += "</p><p>The road continued.</p></section>";
        }
        book += "</body></FictionBook>";
        QFile file(path);
        require(file.open(QIODevice::WriteOnly) && file.write(book) == book.size(), u"Authored book write"_s);
        file.close();
        controller->importLocalBook(QUrl::fromLocalFile(path));
        waitFor([&] { return controller->property("readerOpen").toBool()
            && !controller->property("readerBusy").toBool()
            && controller->property("readerChapterId").toString() == u"1"; }, u"Authored book"_s);
        settle();
    };
    const auto translated = [&]
    {
        waitFor([&] { return !controller->property("readerBusy").toBool()
            && !controller->property("readerTranslationBusy").toBool()
            && controller->property("readerTranslationRequested").toBool()
            && content(controller).startsWith(controller->property("readerTranslationLanguage").toString() + u':');
        }, u"Foreground translation"_s);
        require(!controller->property("readerTranslationError").toBool(), u"Foreground translation error"_s);
        require(controller->property("readerTranslationErrorText").toString().isEmpty(),
            u"Text reader inherited a manga decline reason"_s);
    };
    const auto nextTurn = [&](const int count, const QString &chapter)
    {
        QJsonObject running;
        waitFor([&]
        {
            const auto recorded = turns(native);
            if (recorded.size() != count)
                return false;
            for (const auto &turn : recorded)
            {
                if (turn[u"chapter_id"_s].toString() == chapter && !exited(turn))
                {
                    running = turn;
                    return true;
                }
            }
            return false;
        }, u"Controlled translation turn"_s);
        return running;
    };
    auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, u"reader-text-ahead"_s);
    const auto closeDatabase = qScopeGuard([&]
    {
        database.close();
        database = {};
        QSqlDatabase::removeDatabase(u"reader-text-ahead"_s);
    });
    database.setDatabaseName(profileDir + u"/content-library.sqlite"_s);
    require(database.open(), database.lastError().text());

    import(u"Read ahead"_s);
    require(turns(native).isEmpty(), u"Original reading started AI"_s);
    hold(native, u"2"_s, true);
    require(controller->setReaderTextTranslation(true), u"Enable translation"_s);
    translated();
    const auto second = nextTurn(2, u"2"_s);
    require(second[u"context"_s].toObject()[u"terms"_s].toArray().size() == 1
        && second[u"context"_s].toObject()[u"facts"_s].toArray().size() == 1,
        u"Read ahead lost accepted names or gender"_s);
    const QString firstText = content(controller);
    const QString firstAnchor = controller->property("readerTextAnchor").toString();
    const auto history = scalar(database, u"SELECT entry FROM library LIMIT 1"_s);
    hold(native, u"2"_s, false);
    waitFor([&] { return scalar(database, u"SELECT count(*) FROM translated_content"_s).toInt() == 2; }, u"Ahead cache"_s);
    settle();
    require(turns(native).size() == 2 && content(controller) == firstText
        && controller->property("readerChapterId").toString() == u"1"
        && controller->property("readerTextAnchor").toString() == firstAnchor
        && scalar(database, u"SELECT entry FROM library LIMIT 1"_s) == history,
        u"Background changed visible reading, history or exceeded one chapter"_s);
    qInfo("PASS reader_text_ahead bounded context history original_opt_in");

    hold(native, u"3"_s, true);
    controller->selectChapter(u"2"_s);
    translated();
    const auto third = nextTurn(3, u"3"_s);
    controller->selectChapter(u"3"_s);
    waitFor([&] { return !controller->property("readerBusy").toBool(); }, u"Promoted original"_s);
    require(controller->property("readerTranslationBusy").toBool() && turns(native).size() == 3,
        u"Promotion duplicated or lost its running job"_s);
    hold(native, u"3"_s, false);
    translated();
    waitFor([&] { return turns(native).size() == 4; }, u"Next after promotion"_s);
    require(exited(third), u"Promoted peer was not reaped"_s);
    translated();
    controller->setReaderTextTranslation(false);
    require(!content(controller).startsWith(u"ru:"_s), u"Original not restored"_s);
    controller->selectChapter(u"1"_s);
    require(controller->setReaderTextTranslation(true), u"Translation intent while original loads"_s);
    translated();
    settle();
    require(turns(native).size() == 4, u"Cached foreground or next chapter repeated AI"_s);
    qInfo("PASS reader_text_ahead promotion cache no_duplicate_ai");

    hold(native, u"2"_s, true);
    require(controller->setReaderTranslationLanguage(u"en"_s), u"English target"_s);
    translated();
    const auto englishAhead = nextTurn(6, u"2"_s);
    require(controller->setReaderTranslationLanguage(u"ru"_s), u"Cancel English target"_s);
    translated();
    waitFor([&] { return exited(englishAhead); }, u"Language cancellation"_s);
    require(controller->setReaderTranslationLanguage(u"en"_s), u"Restore English target"_s);
    translated();
    const auto supersededAhead = nextTurn(7, u"2"_s);
    controller->selectChapter(u"4"_s);
    translated();
    waitFor([&] { return exited(supersededAhead); }, u"Foreground priority cancellation"_s);
    require(turns(native).size() == 8, u"Foreground did not supersede unrelated read ahead"_s);
    require(controller->setReaderTranslationLanguage(u"ru"_s), u"Russian target"_s);
    translated();
    controller->selectChapter(u"1"_s);
    translated();
    require(controller->loadReaderTranslationOptions(), u"Translation options"_s);
    waitFor([&] { return !controller->property("readerTranslationOptionsLoading").toBool(); }, u"Model catalog"_s);
    require(controller->setReaderTranslationModel(u"fixture-alt"_s), u"Model selection"_s);
    translated();
    const auto modelAhead = nextTurn(10, u"2"_s);
    require(controller->setReaderTranslationEffort(u"medium"_s), u"Effort selection"_s);
    translated();
    waitFor([&] { return exited(modelAhead); }, u"Model cancellation"_s);
    const auto effortAhead = nextTurn(12, u"2"_s);
    controller->setReaderTextTranslation(false);
    waitFor([&] { return exited(effortAhead); }, u"Original cancellation"_s);
    controller->setReaderTextTranslation(true);
    translated();
    const auto closingAhead = nextTurn(13, u"2"_s);
    controller->closeReader();
    waitFor([&] { return exited(closingAhead); }, u"Close cancellation"_s);
    import(u"Another work"_s);
    controller->setReaderTextTranslation(true);
    translated();
    const auto replacedAhead = nextTurn(15, u"2"_s);
    import(u"Final work"_s);
    waitFor([&] { return exited(replacedAhead); }, u"Work cancellation"_s);
    require(!controller->property("readerTranslationRequested").toBool(), u"New work inherited translation"_s);
    qInfo("PASS reader_text_ahead language model effort original close work_cancel");

    controller->setReaderTextTranslation(true);
    translated();
    const auto stale = nextTurn(17, u"2"_s);
    const int cachedBefore = scalar(database, u"SELECT count(*) FROM translated_content"_s).toInt();
    QSqlQuery revision(database);
    require(revision.exec(u"UPDATE translation_context SET context = json_set(context, '$.revision', revision + 1), revision = revision + 1"_s),
        revision.lastError().text());
    hold(native, u"2"_s, false);
    waitFor([&] { return exited(stale); }, u"Stale context result"_s);
    settle();
    require(turns(native).size() == 17
        && scalar(database, u"SELECT count(*) FROM translated_content"_s).toInt() == cachedBefore
        && !controller->property("readerTranslationError").toBool()
        && controller->property("readerChapterId").toString() == u"1",
        u"Stale read ahead was accepted, retried or disturbed foreground"_s);
    for (const auto &turn : turns(native))
        waitFor([&] { return exited(turn); }, u"Owned peer reaping"_s);
    require(view->grabFramebuffer().save(profileDir + u"/reader-text-ahead.png"_s), u"Qt render capture"_s);
    controller->closeReader();
    qInfo("PASS reader_text_ahead stale_revision no_paid_retry");
}
