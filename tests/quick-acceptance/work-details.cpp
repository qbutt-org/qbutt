// SPDX-License-Identifier: GPL-2.0-or-later
#include "work-details.h"

#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QDebug>
#include <QDir>
#include <QDesktopServices>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QQmlEngine>
#include <QQuickItem>
#include <QScopeGuard>
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QTemporaryDir>
#include <QThreadPool>
#include <QTimer>
#include <QUrl>
#include <QVariant>
#include <QWheelEvent>

#include "gui/quick/contentview.h"
#include "qbutt_quick/src/controller.cxxqt.h"

using namespace Qt::Literals::StringLiterals;

namespace
{
    const QString shortDescription = u"Instructions from the other publication."_s;

    class BrowserLinks : public QObject
    {
        Q_OBJECT

    public:
        QList<QUrl> opened;

    public slots:
        void open(const QUrl &url)
        {
            opened.append(url);
        }
    };

    void require(const bool condition, const QString &message)
    {
        if (!condition)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const std::function<bool()> &ready, const QString &operation)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < 10'000))
        {
            QEventLoop loop;
            QTimer::singleShot(20, &loop, &QEventLoop::quit);
            loop.exec();
        }
        require(ready(), operation + u" timed out"_s);
    }

    void readyLibrary(Qbutt::ContentController &controller)
    {
        waitFor([&]
        {
            const auto state = controller.property("libraryState").value<Qbutt::Library::LibraryState>();
            return (state == Qbutt::Library::LibraryState::Ready) || (state == Qbutt::Library::LibraryState::Failed);
        }, u"Generated details library"_s);
        require(controller.property("libraryState").value<Qbutt::Library::LibraryState>()
            == Qbutt::Library::LibraryState::Ready, controller.property("libraryErrorText").toString());
    }

    void settled(Qbutt::ContentController &controller)
    {
        waitFor([&] { return !controller.property("libraryPending").toBool(); }, u"Details preference write"_s);
        require(controller.property("libraryErrorText").toString().isEmpty(), controller.property("libraryErrorText").toString());
    }

    QString longDescription()
    {
        QString description = u"Installation <plain text>\n"_s;
        for (int step = 1; step <= 45; ++step)
            description += u"%1. Read the generated legal fixture instructions in order.\n"_s.arg(step);
        return description + u"Cooperative play\n- Create a local room.\n- Invite a friend."_s;
    }

    QJsonObject generatedEntry(const QString &slug, const bool missingPublication)
    {
        const QString hash(40, u'a');
        const bool forum = slug == u"forum-topic"_s;
        const QJsonObject primary {{"source", forum ? "ru_tracker" : "thelastgame"}, {"id", forum ? u"9542"_s : slug}};
        const QJsonObject secondary {{"source", forum ? "nnmclub" : "fitgirl"}, {"id", forum ? u"9543"_s : slug}};
        const auto variant = [](const QJsonObject &publication, const QString &hash)
        {
            return QJsonObject {
                {"reference", QJsonObject {{"publication", publication},
                    {"selector", QJsonObject {{"torrent_hash", QJsonObject {{"info_hash", hash}}}}}}},
                {"title", "Generated legal variant"}, {"transport", "torrent"}, {"info_hash", hash},
                {"languages", QJsonArray()}, {"reported_video_ads", false}
            };
        };
        const auto work = [&](const QJsonObject &publication, const QString &description)
        {
            QString url = u"https://example.com/generated/%1/%2/"_s.arg(publication.value("source").toString(), slug);
            if (forum)
            {
                url = u"https://%1/forum/viewtopic.php?t=%2"_s.arg(publication == primary ? u"rutracker.org"_s : u"nnmclub.to"_s,
                    publication.value("id").toString());
            }
            else if (slug == u"nonpublic-page"_s)
                url = u"https://localhost/generated/"_s;
            QJsonArray variants {variant(publication, hash)};
            if (publication == primary)
                variants.append(variant(publication, QString(40, u'b')));
            const QString author = (publication == primary) ? u"First reader"_s : u"Second reader"_s;
            const QJsonArray notes {QJsonObject {
                {"text", u"Comment from %1: <plain text>"_s.arg(author)}, {"author", author},
                {"url", url + u"#comment-1"_s}
            }};
            return QJsonObject {
                {"reference", publication}, {"title", "Generated legal work"}, {"category", forum ? "movies" : "games"},
                {"url", url}, {"description", description},
                {"notes", forum ? QJsonArray() : notes},
                {"aliases", QJsonArray()}, {"identifiers", QJsonObject()},
                {"needs_inspection", false}, {"variants", variants}
            };
        };
        QJsonArray publications {work(primary, longDescription())};
        if (!missingPublication)
            publications.append(work(secondary, shortDescription));
        return {
            {"group", QJsonObject {{"anchor", primary}, {"references", QJsonArray {primary, secondary}},
                {"publications", publications}}},
            {"preferences", QJsonObject {{"selected", variant(missingPublication ? secondary : primary, hash).value("reference")},
                {"filters", QJsonObject()}}},
            {"favorite", true}
        };
    }

    struct Screen
    {
        Content::ContentView &view;
        Qbutt::ContentController *controller;

        explicit Screen(Content::ContentView &contentView)
            : view {contentView}
            , controller {view.findChild<Qbutt::ContentController *>()}
        {
            waitFor([&] { return view.status() != QQuickWidget::Loading; }, u"Details production QML"_s);
            require(controller && (view.status() == QQuickWidget::Ready), u"Details production view failed"_s);
            readyLibrary(*controller);
            controller->setListMode(Qbutt::Search::ListMode::Favorites);
        }

        QObject *object(const QString &name) const
        {
            auto *result = view.rootObject()->findChild<QObject *>(name);
            require(result, u"Details control missing: "_s + name);
            return result;
        }

        void select(const QString &slug)
        {
            controller->selectWork(u"thelastgame:"_s + slug);
            require(!controller->property("inspecting").toBool(), u"Details fixture requested public inspection"_s);
        }

        void description(const QString &expected, const QString &source, const bool truncated)
        {
            const QVariantList notes = controller->property("selectedPublicationNotes").toList();
            waitFor([&]
            {
                return object(u"contentWorkDescription"_s)->property("truncated").toBool() == truncated
                    && object(u"contentWorkDetails"_s)->property("visible").toBool() == (truncated || !notes.isEmpty());
            }, u"Details summary layout"_s);
            require(controller->property("selectedWorkDescription").toString() == expected
                && controller->property("selectedPublicationSource").toString() == source
                && object(u"contentWorkDescription"_s)->property("text").toString() == expected
                && object(u"contentWorkDescription"_s)->property("maximumLineCount").toInt() == 3,
                u"Description substituted another publication or expanded the summary"_s);
            if (source.isEmpty())
                require(notes.isEmpty(), u"Missing publication retained another source's comments"_s);
            else
            {
                const QString author = (source == u"TheLastGame"_s) ? u"First reader"_s : u"Second reader"_s;
                require((notes.size() == 1) && (notes.first().toMap().value(u"author"_s).toString() == author)
                    && (notes.first().toMap().value(u"text"_s).toString() == u"Comment from %1: <plain text>"_s.arg(author))
                    && notes.first().toMap().value(u"hasSourceLink"_s).toBool(),
                    u"Selected publication comments or authors were mixed"_s);
                require(!controller->openPublicationNote(u"another-publication"_s, 0)
                    && !controller->openPublicationNote(controller->property("selectedPublicationId").toString(), -1)
                    && !controller->openPublicationNote(controller->property("selectedPublicationId").toString(), 1),
                    u"Invalid comment selection opened a source link"_s);
            }
        }

        void page(const QString &expected, BrowserLinks &browser)
        {
            auto *button = object(u"contentPublicationPage"_s);
            require(controller->property("canOpenPublication").toBool() == !expected.isEmpty()
                && button->property("enabled").toBool() == !expected.isEmpty()
                && button->property("text").toString() == QCoreApplication::translate("WorkPane", "Source page"),
                u"Publication page action has the wrong availability or label"_s);
            const qsizetype previous = browser.opened.size();
            const QString publication = controller->property("selectedPublicationId").toString();
            require(!controller->openPublication(u"stale-publication"_s), u"Stale publication opened a page"_s);
            if (expected.isEmpty())
                require(!controller->openPublication(publication), u"Unavailable publication opened a page"_s);
            else
            {
                require(QMetaObject::invokeMethod(button, "clicked") && browser.opened.size() == previous + 1
                    && browser.opened.last() == QUrl(expected), u"Publication page opened another source or acquisition URL"_s);
            }
            require(browser.opened.size() == previous + (expected.isEmpty() ? 0 : 1),
                u"Rejected publication navigated the browser"_s);
        }

        QQuickItem *document() const
        {
            auto *result = qobject_cast<QQuickItem *>(object(u"contentWorkDetailsDocument"_s));
            require(result, u"Details flickable document is missing"_s);
            return result;
        }

        void wheel(const int delta)
        {
            auto *flickable = document();
            const QPointF position = flickable->mapToScene(flickable->boundingRect().center());
            QWheelEvent event(position, view.mapToGlobal(position.toPoint()), {}, {0, delta},
                Qt::NoButton, Qt::NoModifier, Qt::NoScrollPhase, false);
            QCoreApplication::sendEvent(&view, &event);
            QElapsedTimer elapsed;
            elapsed.start();
            do
            {
                const qreal offset = flickable->property("contentY").toReal();
                const qreal maximum = qMax<qreal>(0, flickable->property("contentHeight").toReal() - flickable->height());
                auto *text = qobject_cast<QQuickItem *>(object(u"contentWorkDetailsText"_s));
                require((offset >= -1) && (offset <= maximum + 1)
                    && text && text->mapRectToItem(flickable, text->boundingRect()).intersects(flickable->boundingRect()),
                    u"Fast wheel left a blank or overscrolled details viewport"_s);
                QEventLoop loop;
                QTimer::singleShot(20, &loop, &QEventLoop::quit);
                loop.exec();
            } while (elapsed.elapsed() < 200);
        }

        void openDetails(const bool fastWheel = false)
        {
            auto *button = object(u"contentWorkDetails"_s);
            require(button->property("text").toString() == QCoreApplication::translate("WorkPane", "Details")
                && QMetaObject::invokeMethod(button, "clicked"), u"Details action failed"_s);
            QObject *modal = nullptr;
            if (fastWheel)
            {
                waitFor([&] { return document()->height() > 0; }, u"Details wheel viewport"_s);
                wheel(-720);
            }
            waitFor([&]
            {
                modal = view.rootObject()->findChild<QObject *>(u"contentWorkDetailsModal"_s);
                return modal && modal->property("opened").toBool();
            }, u"Details modal entry"_s);
            require(object(u"contentWorkDetailsSource"_s)->property("text").toString()
                    == controller->property("selectedPublicationSource").toString()
                && object(u"contentWorkDetailsText"_s)->property("text").toString()
                    == controller->property("selectedWorkDescription").toString()
                && object(u"contentWorkDetailsText"_s)->property("textFormat").toInt() == 0
                && object(u"contentWorkDetailsText"_s)->property("readOnly").toBool()
                && object(u"contentWorkDetailsText"_s)->property("selectByMouse").toBool()
                && object(u"contentWorkDetailsClose"_s)->property("text").toString()
                    == QCoreApplication::translate("WorkDetailsModal", "Close"),
                u"Details lost its exact source, plain text or localized action"_s);
            auto *content = modal->property("contentItem").value<QQuickItem *>();
            require(content && view.rootObject()->boundingRect().contains(content->mapRectToScene(content->boundingRect())),
                u"Details content extends outside the product window"_s);
            auto *flickable = document();
            if (object(u"contentWorkDescription"_s)->property("truncated").toBool())
            {
                waitFor([&] { return flickable->property("contentHeight").toReal() > flickable->height(); },
                    u"Long details layout"_s);
            }
            const QVariantList notes = controller->property("selectedPublicationNotes").toList();
            QQuickItem *noteText = nullptr;
            QList<QQuickItem *> candidates {document()};
            for (qsizetype index = 0; index < candidates.size(); ++index)
            {
                auto *candidate = candidates[index];
                if (candidate->objectName() == u"contentPublicationNoteText"_s)
                {
                    noteText = candidate;
                    break;
                }
                candidates.append(candidate->childItems());
            }
            require(noteText, u"Comment visual item is missing"_s);
            require(noteText->property("text").toString()
                    == notes.first().toMap().value(u"text"_s).toString()
                && noteText->property("textFormat").toInt() == 0,
                u"Comment body was missing or interpreted as markup"_s);
        }

        void closed()
        {
            waitFor([&] { return !object(u"contentWorkDetailsModal"_s)->property("visible").toBool(); },
                u"Details modal exit"_s);
        }

        void escape()
        {
            QKeyEvent press(QEvent::KeyPress, Qt::Key_Escape, Qt::NoModifier);
            QCoreApplication::sendEvent(&view, &press);
            QKeyEvent release(QEvent::KeyRelease, Qt::Key_Escape, Qt::NoModifier);
            QCoreApplication::sendEvent(&view, &release);
            closed();
        }
    };
}

void exerciseWorkDetails()
{
    BrowserLinks browser;
    QDesktopServices::setUrlHandler(u"https"_s, &browser, "open");
    const auto restoreBrowser = qScopeGuard([] { QDesktopServices::unsetUrlHandler(u"https"_s); });
    QTemporaryDir profile;
    require(profile.isValid(), u"Details profile creation failed"_s);
    const auto drain = qScopeGuard([]
    {
        if (!QThreadPool::globalInstance()->waitForDone(100'000))
            qFatal("Details fixture owners did not drain");
    });
    {
        Qbutt::ContentController creator;
        creator.initLibrary(profile.path());
        readyLibrary(creator);
        for (const QString &source : {u"thelastgame"_s, u"fitgirl"_s, u"rutracker"_s, u"nnmclub"_s})
        {
            require(creator.setSourceEnabled(source, false), u"Details fixture source disable failed"_s);
            settled(creator);
        }
    }
    require(QThreadPool::globalInstance()->waitForDone(100'000), u"Details initial owner did not drain"_s);
    const QString connection = u"generated-work-details"_s;
    auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, connection);
    const auto closeDatabase = qScopeGuard([&]
    {
        database.close();
        database = QSqlDatabase();
        QSqlDatabase::removeDatabase(connection);
    });
    database.setDatabaseName(profile.filePath(u"content-library.sqlite"_s));
    require(database.open(), database.lastError().text());
    for (const QString &slug : {u"generated-details"_s, u"other-details"_s, u"missing-publication"_s,
        u"nonpublic-page"_s, u"forum-topic"_s})
    {
        const QJsonObject entry = generatedEntry(slug, slug == u"missing-publication"_s);
        const QJsonObject anchor = entry.value("group").toObject().value("anchor").toObject();
        QSqlQuery insert(database);
        require(insert.prepare(u"INSERT INTO library VALUES (?, ?, ?)"_s), insert.lastError().text());
        insert.addBindValue(u"{\"source\":\"%1\",\"id\":\"%2\"}"_s.arg(anchor.value("source").toString(), anchor.value("id").toString()));
        insert.addBindValue(QString::fromUtf8(QJsonDocument(entry).toJson(QJsonDocument::Compact)));
        insert.addBindValue(1);
        require(insert.exec(), insert.lastError().text());
    }
    int warnings = 0;
    QString first;
    QString second;
    QString alternative;
    {
        Content::ContentView view(profile.path());
        view.show();
        QObject::connect(view.engine(), &QQmlEngine::warnings, &view, [&](const QList<QQmlError> &) { ++warnings; });
        Screen screen(view);
        screen.controller->setBrowserChoice(u"auto"_s);
        screen.page({}, browser);
        screen.select(u"generated-details"_s);
        first = screen.controller->property("selectedVariantId").toString();
        auto *variants = qobject_cast<QAbstractItemModel *>(screen.controller->property("variants").value<QObject *>());
        require(variants && (variants->rowCount() == 3), u"Details fixture lost a publication or alternative"_s);
        const int idRole = variants->roleNames().key("variantId", -1);
        const int sourceRole = variants->roleNames().key("source", -1);
        require((idRole >= 0) && (sourceRole >= 0), u"Details variant identity or source role missing"_s);
        for (int row = 0; row < variants->rowCount(); ++row)
        {
            const QString id = variants->data(variants->index(row, 0), idRole).toString();
            if (variants->data(variants->index(row, 0), sourceRole).toString() == u"FitGirl"_s)
                second = id;
            else if (id != first)
                alternative = id;
        }
        require(!first.isEmpty() && !second.isEmpty() && !alternative.isEmpty(), u"Details exact variants are missing"_s);
        for (const QSize &size : {QSize(1280, 720), QSize(1440, 900)})
        {
            view.resize(size);
            screen.controller->selectVariant(first);
            screen.page(u"https://example.com/generated/thelastgame/generated-details/"_s, browser);
            auto *pageButton = qobject_cast<QQuickItem *>(screen.object(u"contentPublicationPage"_s));
            const QPointF point = pageButton->mapToScene(QPointF(pageButton->width() / 2, pageButton->height() / 2));
            const QPointF global = view.mapToGlobal(point.toPoint());
            QMouseEvent move(QEvent::MouseMove, point, global, Qt::NoButton, Qt::NoButton, Qt::NoModifier);
            QCoreApplication::sendEvent(&view, &move);
            waitFor([&] { return pageButton->property("hovered").toBool(); }, u"Source page hover"_s);
            const QString renderDirectory = qEnvironmentVariable("QBUTT_WORK_RENDER_DIR");
            if (!renderDirectory.isEmpty())
            {
                require(QDir().mkpath(renderDirectory), u"Could not create source page render directory"_s);
                require(view.grabFramebuffer().save(QDir(renderDirectory).filePath(
                    u"source-page-%1.png"_s.arg(size.width()))), u"Could not render source page action"_s);
            }
            const qsizetype previousLinks = browser.opened.size();
            QMouseEvent press(QEvent::MouseButtonPress, point, global, Qt::LeftButton, Qt::LeftButton, Qt::NoModifier);
            QCoreApplication::sendEvent(&view, &press);
            QMouseEvent release(QEvent::MouseButtonRelease, point, global, Qt::LeftButton, Qt::NoButton, Qt::NoModifier);
            QCoreApplication::sendEvent(&view, &release);
            require(browser.opened.size() == previousLinks + 1, u"Source page pointer action did not open its URL"_s);
            screen.description(longDescription(), u"TheLastGame"_s, true);
            screen.openDetails(true);
            screen.wheel(-720);
            const qreal bottom = screen.document()->property("contentHeight").toReal() - screen.document()->height();
            require(screen.document()->setProperty("contentY", bottom) && (bottom > 0),
                u"Details could not reach its final instructions"_s);
            screen.escape();
            screen.openDetails();
            require(qAbs(screen.document()->property("contentY").toReal() - bottom) < 1,
                u"Escape and reopen reset the same publication scroll position"_s);
            screen.controller->selectVariant(alternative);
            screen.page(u"https://example.com/generated/thelastgame/generated-details/"_s, browser);
            require(screen.object(u"contentWorkDetailsModal"_s)->property("opened").toBool()
                && (qAbs(screen.document()->property("contentY").toReal() - bottom) < 1),
                u"Another variant of the same publication closed or reset its details"_s);
            require(QMetaObject::invokeMethod(screen.object(u"contentWorkDetailsClose"_s), "clicked"),
                u"Details close action failed"_s);
            screen.closed();
            screen.openDetails();
            require(qAbs(screen.document()->property("contentY").toReal() - bottom) < 1,
                u"Close and reopen reset the same publication scroll position"_s);
            screen.controller->selectVariant(second);
            require(!screen.controller->openPublication(u"thelastgame:generated-details"_s),
                u"Previous publication remained actionable after selection"_s);
            screen.page(u"https://example.com/generated/fitgirl/generated-details/"_s, browser);
            screen.closed();
            require(qAbs(screen.document()->property("contentY").toReal()) < 1,
                u"Another publication retained the previous scroll position"_s);
            screen.description(shortDescription, u"FitGirl"_s, false);
            screen.openDetails();
            screen.escape();
            settled(*screen.controller);
            screen.select(u"generated-details"_s);
            screen.page(u"https://example.com/generated/fitgirl/generated-details/"_s, browser);
            screen.description(shortDescription, u"FitGirl"_s, false);
            screen.controller->selectVariant(first);
            screen.description(longDescription(), u"TheLastGame"_s, true);
            screen.openDetails();
            require(qAbs(screen.document()->property("contentY").toReal()) < 1,
                u"Returning to another publication did not reset its details"_s);
            screen.select(u"other-details"_s);
            screen.page(u"https://example.com/generated/thelastgame/other-details/"_s, browser);
            screen.closed();
            screen.description(longDescription(), u"TheLastGame"_s, true);
            require(screen.controller->property("selectedPublicationId").toString() == u"thelastgame:other-details"_s
                && (qAbs(screen.document()->property("contentY").toReal()) < 1),
                u"Equal text and source hid another publication identity"_s);
            screen.openDetails();
            screen.select(u"missing-publication"_s);
            screen.page({}, browser);
            screen.closed();
            screen.description({}, {}, false);
            require(screen.controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
                == Qbutt::Search::VariantSelectionState::Unavailable, u"Missing publication was silently replaced"_s);
            screen.select(u"nonpublic-page"_s);
            screen.page({}, browser);
            screen.select(u"generated-details"_s);
            screen.controller->selectVariant(second);
            settled(*screen.controller);
        }
        screen.controller->selectWork(u"rutracker:9542"_s);
        screen.page(u"https://rutracker.org/forum/viewtopic.php?t=9542"_s, browser);
        for (int row = 0; row < variants->rowCount(); ++row)
        {
            if (variants->data(variants->index(row, 0), sourceRole).toString() == u"NNM-Club"_s)
            {
                screen.controller->selectVariant(variants->data(variants->index(row, 0), idRole).toString());
                break;
            }
        }
        screen.page(u"https://nnmclub.to/forum/viewtopic.php?t=9543"_s, browser);
    }
    require(QThreadPool::globalInstance()->waitForDone(100'000), u"Details view owner did not drain"_s);
    {
        Content::ContentView view(profile.path());
        view.resize(1280, 720);
        view.show();
        Screen screen(view);
        screen.controller->setBrowserChoice(u"auto"_s);
        screen.select(u"generated-details"_s);
        screen.page(u"https://example.com/generated/fitgirl/generated-details/"_s, browser);
        screen.description(shortDescription, u"FitGirl"_s, false);
        require(screen.controller->property("selectedVariantId").toString() == second,
            u"Reopening replaced the exact publication preference"_s);
    }
    require(warnings == 0, u"Details production QML emitted warnings"_s);
    qInfo("PASS work_details selected_publication=exact publication_page=exact_and_guarded publication_comments=exact comment_links=guarded missing_publication=empty summary=three_lines details=truncated_or_comments plain_text=complete wheel_bounds=ok escape_reopen=position_preserved variant_same_publication=position_preserved publication_change=closed_reset refresh=preserved reopen=exact sizes=1280x720,1440x900");
}

#include "work-details.moc"
