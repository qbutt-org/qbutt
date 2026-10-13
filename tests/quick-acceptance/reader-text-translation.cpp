// SPDX-License-Identifier: GPL-2.0-or-later
#include "reader-text-translation.h"

#include <algorithm>
#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QDebug>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QImage>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMetaObject>
#include <QQuickItem>
#include <QScopeGuard>
#include <QSize>
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

    void waitFor(const std::function<bool()> &condition, const QString &operation, const int timeoutMs = 20'000)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        QEventLoop loop;
        QTimer timer;
        timer.setInterval(25);
        QObject::connect(&timer, &QTimer::timeout, &loop, [&]
        {
            if (condition() || elapsed.elapsed() >= timeoutMs)
                loop.quit();
        });
        if (!condition())
        {
            timer.start();
            loop.exec();
        }
        require(condition(), operation + u" timed out"_s);
    }

    QQuickItem *readerItem(Content::ContentView *view)
    {
        auto *loader = view->rootObject()->findChild<QObject *>(u"contentReaderLoader"_s);
        return loader ? loader->property("item").value<QQuickItem *>() : nullptr;
    }

    QQuickItem *control(QQuickItem *reader, const QString &name)
    {
        auto *item = reader ? reader->findChild<QQuickItem *>(name) : nullptr;
        require(item, u"Reader control is missing: "_s + name);
        return item;
    }

    void activate(QQuickItem *choice, const int index)
    {
        require(choice->setProperty("currentIndex", index)
            && QMetaObject::invokeMethod(choice, "activated", Q_ARG(int, index)),
            u"Reader selection could not be activated"_s);
    }

    QAbstractItemModel *textBlocks(Qbutt::ContentController *controller)
    {
        auto *blocks = qobject_cast<QAbstractItemModel *>(
            controller->property("readerTextBlocks").value<QObject *>());
        require(blocks && blocks->rowCount() > 1, u"Native text blocks are missing"_s);
        return blocks;
    }

    QString textAt(QAbstractItemModel *blocks, const int row)
    {
        const int role = blocks->roleNames().key("plainText", -1);
        require(role >= 0, u"Native plain text role is missing"_s);
        return blocks->data(blocks->index(row, 0), role).toString();
    }
}

void exerciseReaderTextTranslation(Content::ContentView *view, const QString &profileDir)
{
    auto *controller = view->findChild<Qbutt::ContentController *>();
    require(controller, u"Text translation controller is missing"_s);
    waitFor([&]
    {
        return controller->property("libraryState").value<Qbutt::Library::LibraryState>()
            == Qbutt::Library::LibraryState::Ready;
    }, u"Text translation library"_s);
    const auto capture = [&](const QString &name, const QSize &size)
    {
        view->resize(size);
        QEventLoop frame;
        QTimer::singleShot(150, &frame, &QEventLoop::quit);
        frame.exec();
        const QImage image = view->grabFramebuffer();
        require(image.size() == size && image.save(profileDir + u"/"_s + name),
            u"Text reader render could not be saved at "_s + QString::number(size.width())
                + u"x"_s + QString::number(size.height()));
    };

    // Both chapters are original, generated text. Russian past tense makes the
    // first chapter's gender ambiguous until the second chapter is inspected.
    const QByteArray book = R"(<?xml version="1.0" encoding="utf-8"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0">
<description><title-info><genre>prose</genre><author><first-name>Fixture</first-name>
<last-name>Author</last-name></author><book-title>Ren's journey</book-title>
<lang>ja</lang></title-info></description>
<body>
  <section><title><p>森への帰還</p></title>
    <p>レンは森に戻った。</p><p>レンは静かに笑った。</p>
  </section>
  <section><title><p>姉妹</p></title>
    <p>レンは妹に言った。「私はあなたの姉です」</p>
  </section>
</body></FictionBook>)";
    const QString path = profileDir + u"/translation.fb2"_s;
    QFile file(path);
    require(file.open(QIODevice::WriteOnly) && file.write(book) == book.size(),
        u"Generated translation book could not be written"_s);
    file.close();

    controller->importLocalBook(QUrl::fromLocalFile(path));
    waitFor([&]
    {
        return controller->property("readerOpen").toBool()
            && !controller->property("readerBusy").toBool()
            && controller->property("readerIsText").toBool();
    }, u"Generated text reader"_s);
    require(controller->property("readerChapterCount").toInt() == 2
        && controller->property("readerChapterId").toString() == u"1"_s
        && controller->property("readerErrorText").toString().isEmpty(),
        u"Generated book did not open at its first text chapter"_s);
    auto *blocks = textBlocks(controller);
    int paragraphIndex = -1;
    for (int row = 0; row < blocks->rowCount(); ++row)
    {
        if (textAt(blocks, row).contains(u"レンは森に戻った"_s))
            paragraphIndex = row;
    }
    require(paragraphIndex >= 0, u"Original paragraph was not rendered"_s);
    const QString original = textAt(blocks, paragraphIndex);

    auto *reader = readerItem(view);
    auto *settings = control(reader, u"readerTranslationSettings"_s);
    auto *picker = reader->findChild<QObject *>(u"readerTranslationPicker"_s);
    require(picker, u"QML translation settings picker is missing"_s);
    auto *modelChoice = control(reader, u"readerTranslationModel"_s);
    auto *effortChoice = control(reader, u"readerTranslationEffort"_s);
    auto *translation = control(reader, u"readerTranslation"_s);
    require(QMetaObject::invokeMethod(settings, "clicked"), u"QML translation settings did not open"_s);
    waitFor([&]
    {
        return picker->property("visible").toBool()
            && !controller->property("readerTranslationOptionsLoading").toBool()
            && controller->property("readerTranslationModelCount").toInt() > 0;
    }, u"Installed Codex model list"_s, 120'000);
    require(!controller->property("readerTranslationOptionsError").toBool()
        && modelChoice->isEnabled(), u"Installed Codex models are unavailable in QML"_s);

    int modelIndex = 0;
    for (int index = 0; index < controller->property("readerTranslationModelCount").toInt(); ++index)
    {
        if (controller->readerTranslationModelAt(index) == u"gpt-6.1-sol"_s)
        {
            modelIndex = index;
            break;
        }
    }
    const QString model = controller->readerTranslationModelAt(modelIndex);
    activate(modelChoice, modelIndex + 1); // Index zero is Automatic.
    waitFor([&]
    {
        return !controller->property("readerTranslationOptionsLoading").toBool()
            && controller->property("readerTranslationModel").toString() == model;
    }, u"QML Codex model choice"_s);
    require(effortChoice->isVisible() && effortChoice->isEnabled(),
        u"Reasoning choice did not follow the selected model"_s);

    int effortIndex = 0;
    for (int index = 0; index < controller->property("readerTranslationEffortCount").toInt(); ++index)
    {
        if (controller->readerTranslationEffortAt(index) == u"low"_s)
        {
            effortIndex = index;
            break;
        }
    }
    const QString effort = controller->readerTranslationEffortAt(effortIndex);
    require(!effort.isEmpty(), u"Selected model has no reasoning choices"_s);
    activate(effortChoice, effortIndex);
    waitFor([&]
    {
        return !controller->property("readerTranslationOptionsLoading").toBool()
            && controller->property("readerTranslationEffort").toString() == effort;
    }, u"QML reasoning choice"_s);
    capture(u"translation-options-ru-1280x720.png"_s, QSize(1280, 720));
    require(QMetaObject::invokeMethod(control(reader, u"readerTranslationClose"_s), "clicked"),
        u"QML translation settings did not close"_s);

    auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, u"reader-text-translation"_s);
    const auto closeDatabase = qScopeGuard([&]
    {
        database.close();
        database = {};
        QSqlDatabase::removeDatabase(u"reader-text-translation"_s);
    });
    database.setDatabaseName(profileDir + u"/content-library.sqlite"_s);
    require(database.open(), database.lastError().text());
    QSqlQuery selection(database);
    require(selection.exec(u"SELECT model, effort, explicit FROM translation_selection WHERE target_language = 'ru'"_s)
        && selection.next() && selection.value(0).toString() == model
        && selection.value(1).toString() == effort && selection.value(2).toInt() == 1,
        u"QML model and reasoning choice were not persisted"_s);
    selection.finish();

    activate(translation, 1);
    waitFor([&] { return controller->property("readerTranslationBusy").toBool(); },
        u"Live Codex translation start"_s);
    waitFor([&] { return !controller->property("readerTranslationBusy").toBool(); },
        u"Live Codex translation"_s, 240'000);
    require(controller->property("readerTranslationRequested").toBool()
        && !controller->property("readerTranslationError").toBool(),
        u"Live Codex translation did not reach the reader"_s);
    const QString translated = textAt(textBlocks(controller), paragraphIndex);
    const bool russianText = std::any_of(translated.cbegin(), translated.cend(), [](const QChar character)
    {
        return character.unicode() >= 0x0400 && character.unicode() <= 0x04ff;
    });
    require(!translated.isEmpty() && translated != original && russianText,
        u"Translated Russian paragraph did not reach the native text model"_s);
    capture(u"translated-ru-1440x900.png"_s, QSize(1440, 900));

    QSqlQuery context(database);
    require(context.exec(u"SELECT context FROM translation_context WHERE language = 'ru'"_s)
        && context.next(), u"Accepted translation context was not saved"_s);
    const auto savedContext = QJsonDocument::fromJson(context.value(0).toByteArray()).object();
    const auto facts = savedContext.value(u"facts"_s).toArray();
    const auto terms = savedContext.value(u"terms"_s).toArray();
    context.finish();
    bool rememberedName = false;
    for (const auto &entry : terms)
    {
        const auto term = entry.toObject();
        rememberedName |= term.value(u"source"_s).toString() == u"レン"_s
            && !term.value(u"target"_s).toString().isEmpty();
    }
    bool femaleFromLaterChapter = false;
    for (const auto &entry : facts)
    {
        const auto fact = entry.toObject();
        const auto evidence = fact.value(u"evidence"_s).toObject();
        femaleFromLaterChapter |= fact.value(u"attribute"_s).toString() == u"gender"_s
            && fact.value(u"value"_s).toString() == u"female"_s
            && evidence.value(u"chapter"_s).toObject().value(u"id"_s).toString() == u"2"_s
            && evidence.value(u"quote"_s).toString().contains(u"姉"_s);
    }
    require(rememberedName && femaleFromLaterChapter,
        u"Codex did not preserve the name and later chapter's gender evidence"_s);

    activate(translation, 0);
    require(!controller->property("readerTranslationRequested").toBool()
        && textAt(textBlocks(controller), paragraphIndex) == original,
        u"QML Original choice did not restore the original text"_s);
    capture(u"original-ru-1280x720.png"_s, QSize(1280, 720));
    controller->closeReader();

    const QByteArray originalProgramFiles = qgetenv("ProgramFiles");
    const QByteArray originalLocalAppData = qgetenv("LOCALAPPDATA");
    const auto restoreEnvironment = qScopeGuard([&]
    {
        qputenv("ProgramFiles", originalProgramFiles);
        qputenv("LOCALAPPDATA", originalLocalAppData);
    });
    qputenv("ProgramFiles", profileDir.toUtf8());
    qputenv("LOCALAPPDATA", profileDir.toUtf8());
    controller->openReader();
    waitFor([&]
    {
        return controller->property("readerOpen").toBool()
            && !controller->property("readerBusy").toBool();
    }, u"Original reader reopen"_s);
    require(textAt(textBlocks(controller), paragraphIndex) == original,
        u"Reopened reader did not start with the original chapter"_s);
    reader = readerItem(view);
    translation = control(reader, u"readerTranslation"_s);
    activate(translation, 1);
    waitFor([&] { return !controller->property("readerTranslationBusy").toBool(); },
        u"Cached translation reopen"_s);
    require(controller->property("readerTranslationRequested").toBool()
        && !controller->property("readerTranslationError").toBool()
        && textAt(textBlocks(controller), paragraphIndex) == translated,
        u"Reopened reader did not reuse the exact translation without Codex"_s);
    qInfo().noquote() << u"PASS reader_text_translation model=%1 effort=%2 context=later_chapter original=ok offline_cache=ok"_s
        .arg(model, effort);
}
