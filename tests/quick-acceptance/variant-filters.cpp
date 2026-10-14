// SPDX-License-Identifier: GPL-2.0-or-later
#include "variant-filters.h"
#include "group-entry.h"

#include <functional>
#include <stdexcept>

#include <QApplication>
#include <QColor>
#include <QDebug>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QImage>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QKeyEvent>
#include <QLocale>
#include <QQmlEngine>
#include <QQuickItem>
#include <QScopeGuard>
#include <QSet>
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QTemporaryDir>
#include <QThreadPool>
#include <QTimer>

#include "gui/quick/contentview.h"
#include "gui/quick/qml/variantfiltermodel.h"
#include "qbutt_quick/src/controller.cxxqt.h"

using namespace Qt::Literals::StringLiterals;

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
        }, u"Generated profile initialization"_s);
        require(controller.property("libraryState").value<Qbutt::Library::LibraryState>()
            == Qbutt::Library::LibraryState::Ready, controller.property("libraryErrorText").toString());
    }

    void settled(Qbutt::ContentController &controller)
    {
        waitFor([&] { return !controller.property("libraryPending").toBool(); }, u"Variant preference write"_s);
        require(controller.property("libraryErrorText").toString().isEmpty(), controller.property("libraryErrorText").toString());
    }

    QJsonObject generatedEntry(const int number)
    {
        const QJsonObject publication {{u"source"_s, u"ani_liberty"_s}, {u"id"_s, QString::number(900'000 + number)}};
        QJsonArray variants;
        for (int episode = 1; episode <= 12; ++episode)
        {
            for (int voice = 0; voice < 2; ++voice)
            {
                for (int player = 0; player < 2; ++player)
                {
                    const QString id = u"generated-%1-%2"_s.arg(episode).arg(voice);
                    const QJsonObject selector = player
                        ? QJsonObject {{u"rutube"_s, QJsonObject {
                            {u"episode_id"_s, id},
                            {u"rutube_id"_s, QString::number(episode * 10 + voice, 16).rightJustified(32, u'0')}
                        }}}
                        : QJsonObject {{u"episode"_s, QJsonObject {{u"id"_s, id}, {u"quality"_s, u"p1080"_s}}}};
                    variants.append(QJsonObject {
                        {u"reference"_s, QJsonObject {{u"publication"_s, publication}, {u"selector"_s, selector}}},
                        {u"title"_s, u"Generated episode %1"_s.arg(episode)}, {u"transport"_s, u"video"_s},
                        {u"episode"_s, QString::number(episode)}, {u"quality"_s, u"1080p"_s},
                        {u"translation"_s, voice ? u"Studio Beta"_s : u"Studio Alpha"_s},
                        {u"languages"_s, QJsonArray {voice ? u"en"_s : u"ru"_s}},
                        {u"player"_s, player ? QJsonValue(u"Rutube"_s) : QJsonValue()},
                        {u"size"_s, QJsonValue()}, {u"seeders"_s, QJsonValue()}, {u"info_hash"_s, QJsonValue()},
                        {u"reported_video_ads"_s, false}
                    });
                }
            }
        }
        QJsonObject filters;
        for (const QString &name : {u"episode"_s, u"translation"_s, u"source"_s, u"player"_s,
            u"language"_s, u"transport"_s, u"query"_s})
        {
            filters.insert(name, QJsonValue());
        }
        const QJsonObject work {
                {u"reference"_s, publication}, {u"title"_s, u"Generated series %1"_s.arg(number)},
                {u"category"_s, u"anime"_s}, {u"url"_s, u"https://example.com/series/%1"_s.arg(number)},
                {u"cover"_s, QJsonValue()}, {u"description"_s, QJsonValue()}, {u"year"_s, 2026},
                {u"aliases"_s, QJsonArray()}, {u"identifiers"_s, QJsonObject()},
                {u"needs_inspection"_s, false}, {u"variants"_s, variants}
        };
        return {
            {u"group"_s, GroupEntry::single(work)},
            {u"preferences"_s, QJsonObject {
                {u"selected"_s, variants.first().toObject().value(u"reference"_s)}, {u"filters"_s, filters}
            }},
            {u"favorite"_s, true}, {u"reader_progress"_s, QJsonValue()}, {u"last_opened"_s, QJsonValue()}
        };
    }

    QJsonObject savedEntry(QSqlDatabase &database, const int number)
    {
        QSqlQuery query(database);
        require(query.prepare(u"SELECT entry FROM library WHERE reference = ?"_s), query.lastError().text());
        query.addBindValue(u"{\"source\":\"ani_liberty\",\"id\":\"%1\"}"_s.arg(900'000 + number));
        require(query.exec() && query.next(), query.lastError().text());
        return QJsonDocument::fromJson(query.value(0).toString().toUtf8()).object();
    }

    struct Screen
    {
        Content::ContentView &view;
        Qbutt::ContentController *controller;
        QQuickItem *variants;
        QAbstractItemModel *full;
        Content::VariantFilterModel *proxy;
        QObject operationObserver;
        int contentOperations = 0;

        explicit Screen(Content::ContentView &contentView)
            : view {contentView}
            , controller {view.findChild<Qbutt::ContentController *>()}
        {
            waitFor([&] { return view.status() != QQuickWidget::Loading; }, u"Production ContentView"_s);
            require(controller && (view.status() == QQuickWidget::Ready), u"Production ContentView failed"_s);
            auto *palette = view.rootObject()->findChild<QObject *>(u"contentApplicationPalette"_s);
            require(palette && ((palette->property("window").value<QColor>().lightness() < 128)
                == QCoreApplication::arguments().contains(u"--dark"_s)), u"Production palette does not match the requested appearance"_s);
            readyLibrary(*controller);
            variants = item(u"contentVariants"_s);
            full = qobject_cast<QAbstractItemModel *>(controller->property("variants").value<QObject *>());
            proxy = qobject_cast<Content::VariantFilterModel *>(variants->property("model").value<QObject *>());
            require(full && proxy && (proxy->sourceModel() == full), u"Production variant projection is missing"_s);
            QObject::connect(controller, &Qbutt::ContentController::searchStateChanged, &operationObserver,
                [&] { contentOperations += controller->property("busy").toBool() ? 1 : 0; });
            QObject::connect(controller, &Qbutt::ContentController::inspectingChanged, &operationObserver,
                [&] { contentOperations += controller->property("inspecting").toBool() ? 1 : 0; });
            QObject::connect(controller, &Qbutt::ContentController::resolvingChanged, &operationObserver,
                [&] { contentOperations += controller->property("resolving").toBool() ? 1 : 0; });
            controller->setListMode(Qbutt::Search::ListMode::Favorites);
        }

        QQuickItem *item(const QString &name) const
        {
            auto *result = view.rootObject()->findChild<QQuickItem *>(name);
            require(result, u"Production control missing: "_s + name);
            return result;
        }

        void select(const int number)
        {
            // Prevent cached fixture selection from inspecting a public catalog.
            require(controller->setSourceEnabled(u"aniliberty"_s, false), u"Source disable failed"_s);
            settled(*controller);
            controller->selectWork(u"aniliberty:%1"_s.arg(900'000 + number));
            require(!controller->property("inspecting").toBool(), u"Offline fixture started inspection"_s);
            require(controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
                    == Qbutt::Search::VariantSelectionState::SourceDisabled
                && item(u"contentUnavailableVariant"_s)->isVisible()
                && (item(u"contentUnavailableVariant"_s)->property("text").toString()
                    == QCoreApplication::translate("WorkPane", "Enable this source in Sources."))
                && !item(u"contentDownload"_s)->isEnabled(),
                u"Disabled source was presented as a missing exact variant or remained actionable"_s);
            require(controller->setSourceEnabled(u"aniliberty"_s, true), u"Source enable failed"_s);
            settled(*controller);
            waitFor([&] { return proxy->episodeOptions().size() == 12; }, u"Full variant facets"_s);
            require(full->rowCount() == 48, u"Stored full variant model was lost"_s);
            require(controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
                    == Qbutt::Search::VariantSelectionState::Available
                && !item(u"contentUnavailableVariant"_s)->isVisible(), u"Fixture selection is unavailable"_s);
        }

        void choose(const QString &name, const QVariantMap &option)
        {
            require(QMetaObject::invokeMethod(item(name), "chosen", Q_ARG(QVariant, QVariant(option))),
                u"Production facet selection failed: "_s + name);
        }

        void query(const QString &text)
        {
            auto *field = item(u"contentVariantQuery"_s);
            require(field->setProperty("text", text) && QMetaObject::invokeMethod(field, "textEdited"),
                u"Production variant query failed"_s);
        }

        void count(const int expected, const QString &selected)
        {
            waitFor([&]
            {
                return (proxy->rowCount() == expected) && (variants->property("count").toInt() == expected)
                    && (proxy->selectedRow() == proxy->indexOfId(selected))
                    && (variants->property("currentIndex").toInt() == proxy->selectedRow());
            }, u"Visible variant count and selection %1"_s.arg(expected));
            require((full->rowCount() == 48) && (proxy->episodeOptions().size() == 12)
                && (controller->property("selectedVariantId").toString() == selected),
                u"Filtering changed the full work, its facets or exact selection"_s);
            require(contentOperations == 0, u"Filtering started content work"_s);
            const int row = proxy->indexOfId(selected);
            require(item(u"contentDownload"_s)->isEnabled() == (row >= 0), u"Hidden exact selection stayed actionable"_s);
            require(item(u"contentSaveVideo"_s)->isVisible(), u"Supported video lost Save"_s);
            require(item(u"contentFilteredSelectionNotice"_s)->isVisible() == (row < 0),
                u"Filtered selection notice does not match the visible exact variant"_s);
            settled(*controller);
            visibleRows();
        }

        void visibleRows()
        {
            require(QMetaObject::invokeMethod(variants, "forceLayout"), u"Variant list layout failed"_s);
            QEventLoop loop;
            QTimer::singleShot(150, &loop, &QEventLoop::quit);
            loop.exec();

            auto *content = variants->property("contentItem").value<QQuickItem *>();
            require(content, u"Variant list content is missing"_s);
            QSet<QString> rendered;
            for (int row = 0; row < proxy->rowCount(); ++row)
            {
                QQuickItem *delegate = nullptr;
                require(QMetaObject::invokeMethod(variants, "itemAtIndex", Q_RETURN_ARG(QQuickItem *, delegate), Q_ARG(int, row)),
                    u"Active variant delegate lookup failed"_s);
                if (!delegate || !delegate->isVisible())
                    continue;
                const QRectF bounds = delegate->mapRectToItem(variants, delegate->boundingRect());
                if (!bounds.intersects(variants->boundingRect()))
                    continue;

                const QString identity = delegate->property("variantId").toString();
                const QRectF visible = bounds.intersected(variants->boundingRect());
                const QPointF position = variants->mapToItem(content, visible.center());
                try
                {
                    int geometricRow = -1;
                    require(QMetaObject::invokeMethod(variants, "indexAt", Q_RETURN_ARG(int, geometricRow),
                        Q_ARG(qreal, position.x()), Q_ARG(qreal, position.y())), u"Visible variant index lookup failed"_s);
                    require((geometricRow == row) && !rendered.contains(identity),
                        u"Visible variant is duplicated or drawn at another row: id=%1 row=%2 geometric_row=%3"_s
                            .arg(identity).arg(row).arg(geometricRow));
                    rendered.insert(identity);
                    for (const QByteArray &role : {QByteArray("variantId"), QByteArray("label"), QByteArray("source"),
                        QByteArray("kind"), QByteArray("quality"), QByteArray("sizeText"), QByteArray("seedersText")})
                    {
                        const int roleIndex = proxy->roleNames().key(role, -1);
                        require(roleIndex >= 0, u"Variant model role is missing: "_s + QString::fromLatin1(role));
                        const QString expected = proxy->data(proxy->index(row, 0), roleIndex).toString();
                        const QString actual = delegate->property(role).toString();
                        require(actual == expected, u"Visible variant row %1 role %2 differs: model=%3 delegate=%4"_s
                            .arg(row).arg(QString::fromLatin1(role)).arg(expected, actual));
                    }
                    require(proxy->indexOfId(identity) == row, u"Visible variant identity maps to a different proxy row"_s);
                }
                catch (const std::exception &)
                {
                    qCritical().noquote() << "variant_delegate_mismatch row=" << row << "delegate=" << identity
                        << "expected=" << proxy->idAt(row) << "bounds=" << bounds << "visible=" << visible
                        << "content_position=" << position << "content_y=" << variants->property("contentY");
                    const QString directory = qEnvironmentVariable("QBUTT_VARIANT_FILTER_RENDER_DIR");
                    if (!directory.isEmpty())
                    {
                        require(QDir().mkpath(directory), u"Delegate diagnostic directory creation failed"_s);
                        const QString path = QDir(directory).filePath(QLocale().name() + u"-delegate-failure.png"_s);
                        require(view.grabFramebuffer().save(path), u"Delegate diagnostic render failed"_s);
                        qCritical().noquote() << "variant_delegate_failure_render" << path;
                    }
                    throw;
                }
            }
            require((proxy->rowCount() == 0) || !rendered.isEmpty(), u"Nonempty variant list has no visible delegates"_s);
        }

        void reset()
        {
            require(QMetaObject::invokeMethod(item(u"contentClearVariantFilters"_s), "clicked"), u"Production reset failed"_s);
        }

        void render(const QString &name)
        {
            const QString directory = qEnvironmentVariable("QBUTT_VARIANT_FILTER_RENDER_DIR");
            if (directory.isEmpty())
                return;
            require(QDir().mkpath(directory), u"Render directory creation failed"_s);
            QEventLoop loop;
            QTimer::singleShot(180, &loop, &QEventLoop::quit);
            loop.exec();
            const QString path = QDir(directory).filePath(QLocale().name() + '-' + name + u".png"_s);
            const QImage image = view.grabFramebuffer();
            require(!image.isNull() && image.save(path), u"Production render failed: "_s + path);
            qInfo().noquote() << "variant_filter_render" << path;
        }
    };

    void exerciseChoices(Screen &screen)
    {
        auto *field = screen.item(u"contentEpisodeFilter"_s);
        auto *picker = screen.view.rootObject()->findChild<QObject *>(u"contentEpisodeFilterPicker"_s);
        require(picker && QMetaObject::invokeMethod(field, "clicked"), u"Episode picker did not open"_s);
        waitFor([&] { return picker->property("opened").toBool(); }, u"Episode picker entry"_s);
        auto *choices = screen.item(u"contentEpisodeFilterChoices"_s);
        auto *query = screen.item(u"contentEpisodeFilterQuery"_s);
        require(choices->property("count").toInt() == 13, u"Facet list did not contain all options and reset"_s);
        require(QMetaObject::invokeMethod(choices, "forceLayout"), u"Episode picker layout failed"_s);
        waitFor([&] { return choices->property("contentHeight").toReal() > choices->height(); }, u"Scrollable episode choices"_s);
        const qreal offset = choices->property("contentHeight").toReal() - choices->height();
        require(choices->setProperty("contentY", offset), u"Episode picker scroll failed"_s);
        require(QMetaObject::invokeMethod(picker, "close"), u"Scrolled episode picker did not close"_s);
        waitFor([&] { return !picker->property("visible").toBool(); }, u"Scrolled episode picker exit"_s);
        require(QMetaObject::invokeMethod(field, "clicked"), u"Scrolled episode picker did not reopen"_s);
        waitFor([&] { return picker->property("opened").toBool(); }, u"Scrolled episode picker reopen"_s);
        require(qAbs(choices->property("contentY").toReal() - offset) < 1,
            u"Episode picker lost its list position on reopen"_s);
        require(query->setProperty("text", u"12"_s), u"Episode picker query failed"_s);
        waitFor([&] { return choices->property("count").toInt() == 1; }, u"Episode picker search"_s);
        screen.render(u"episode-picker"_s);
        require(QMetaObject::invokeMethod(picker, "close"), u"Episode picker did not close"_s);
        waitFor([&] { return !picker->property("visible").toBool(); }, u"Episode picker exit"_s);
        require(QMetaObject::invokeMethod(field, "clicked"), u"Episode picker did not reopen"_s);
        waitFor([&] { return picker->property("opened").toBool(); }, u"Episode picker reopen"_s);
        require((query->property("text").toString() == u"12"_s) && (choices->property("count").toInt() == 1),
            u"Episode picker lost its query on reopen"_s);
        require(choices->setProperty("currentIndex", 0), u"Episode picker keyboard selection failed"_s);
        choices->forceActiveFocus();
        QKeyEvent press(QEvent::KeyPress, Qt::Key_Return, Qt::NoModifier);
        QCoreApplication::sendEvent(&screen.view, &press);
        QKeyEvent release(QEvent::KeyRelease, Qt::Key_Return, Qt::NoModifier);
        QCoreApplication::sendEvent(&screen.view, &release);
        waitFor([&] { return !picker->property("visible").toBool(); }, u"Episode picker final exit"_s);
        require((screen.controller->property("variantEpisodeFilter").toString() == u"12"_s)
            && (field->property("value").toString() == u"12"_s), u"Episode picker lost its selected option"_s);
        screen.reset();
    }
}

void exerciseVariantFilters()
{
    QTemporaryDir profile;
    require(profile.isValid(), u"Variant profile creation failed"_s);
    const auto drain = qScopeGuard([]
    {
        if (!QThreadPool::globalInstance()->waitForDone(100'000))
            qFatal("Variant filter fixture owners did not drain");
    });
    {
        Qbutt::ContentController creator;
        creator.initLibrary(profile.path());
        readyLibrary(creator);
    }
    require(QThreadPool::globalInstance()->waitForDone(100'000), u"Initial profile owner did not drain"_s);
    const QString connection = u"production-variant-filters"_s;
    auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, connection);
    const auto closeDatabase = qScopeGuard([&]
    {
        database.close();
        database = QSqlDatabase();
        QSqlDatabase::removeDatabase(connection);
    });
    database.setDatabaseName(profile.filePath(u"content-library.sqlite"_s));
    require(database.open(), database.lastError().text());
    for (int number = 1; number <= 2; ++number)
    {
        QSqlQuery query(database);
        require(query.prepare(u"INSERT INTO library VALUES (?, ?, ?)"_s), query.lastError().text());
        query.addBindValue(u"{\"source\":\"ani_liberty\",\"id\":\"%1\"}"_s.arg(900'000 + number));
        query.addBindValue(QString::fromUtf8(QJsonDocument(generatedEntry(number)).toJson(QJsonDocument::Compact)));
        query.addBindValue(number);
        require(query.exec(), query.lastError().text());
    }
    int warnings = 0;
    QString selected;
    QJsonValue preferences;
    {
        Content::ContentView view(profile.path());
        view.resize(1440, 900);
        view.show();
        QObject::connect(view.engine(), &QQmlEngine::warnings, &view, [&](const QList<QQmlError> &) { ++warnings; });
        Screen screen(view);
        screen.select(1);
        selected = screen.controller->property("selectedVariantId").toString();
        require(!selected.isEmpty(), u"Fixture exact selection is missing"_s);
        require((screen.proxy->translationOptions().size() == 2) && (screen.proxy->providerOptions().size() == 2)
            && (screen.proxy->languageOptions().size() == 2)
            && (screen.proxy->episodeOptions().at(1).toMap().value(u"value"_s).toString() == u"2"_s)
            && (screen.proxy->episodeOptions().last().toMap().value(u"value"_s).toString() == u"12"_s),
            u"Facets lost distinct metadata or natural episode ordering"_s);
        screen.count(48, selected);
        screen.render(u"all-1440"_s);
        exerciseChoices(screen);
        screen.choose(u"contentProviderFilter"_s, {{u"source"_s, u"aniliberty"_s}, {u"player"_s, QString()}});
        screen.count(48, selected);
        screen.choose(u"contentTranslationFilter"_s, {{u"value"_s, u"Studio Beta"_s}});
        screen.count(24, selected);
        screen.choose(u"contentTranslationFilter"_s, {{u"value"_s, u"Studio Alpha"_s}});
        screen.count(24, selected);
        screen.choose(u"contentTranslationFilter"_s, {{u"value"_s, QString()}});
        screen.count(48, selected);
        screen.choose(u"contentProviderFilter"_s, {{u"source"_s, u"aniliberty"_s}, {u"player"_s, u"Rutube"_s}});
        screen.count(24, selected);
        screen.choose(u"contentEpisodeFilter"_s, {{u"value"_s, u"2"_s}});
        screen.count(2, selected);
        screen.choose(u"contentTranslationFilter"_s, {{u"value"_s, u"Studio Beta"_s}});
        screen.count(1, selected);
        screen.choose(u"contentLanguageFilter"_s, {{u"value"_s, u"ru"_s}});
        screen.count(0, selected);
        screen.choose(u"contentLanguageFilter"_s, {{u"value"_s, u"en"_s}});
        screen.count(1, selected);
        screen.query(u"1080 no-such-variant"_s);
        screen.count(0, selected);
        screen.render(u"empty-1440"_s);
        screen.query(u"1080"_s);
        screen.count(1, selected);
        const auto confirmed = savedEntry(database, 1);
        {
            QSqlQuery fault(database);
            require(fault.exec(u"CREATE TRIGGER reject_variant_preferences BEFORE UPDATE ON library "
                "BEGIN SELECT RAISE(ABORT, 'generated variant preference fault'); END"_s), fault.lastError().text());
        }
        screen.query(u"retry query"_s);
        waitFor([&] { return !screen.controller->property("libraryPending").toBool(); }, u"Rejected variant preference write"_s);
        require(screen.controller->property("libraryErrorKind").value<Qbutt::Library::LibraryErrorKind>()
                == Qbutt::Library::LibraryErrorKind::Writing
            && (screen.controller->property("variantQuery").toString() == u"retry query"_s)
            && (screen.controller->property("selectedVariantId").toString() == selected)
            && (savedEntry(database, 1) == confirmed), u"SQL abort changed confirmed metadata or lost current filter intent"_s);
        {
            QSqlQuery fault(database);
            require(fault.exec(u"DROP TRIGGER reject_variant_preferences"_s), fault.lastError().text());
        }
        screen.query(u"retry query"_s);
        screen.count(0, selected);
        require(savedEntry(database, 1).value(u"preferences"_s).toObject().value(u"filters"_s).toObject()
            .value(u"query"_s).toString() == u"retry query"_s, u"Unchanged filter retry did not commit"_s);
        screen.query(u"1080"_s);
        screen.count(1, selected);
        preferences = savedEntry(database, 1).value(u"preferences"_s);
        require(preferences.toObject().value(u"filters"_s).toObject().value(u"query"_s).toString() == u"1080"_s,
            u"Text query was not persisted"_s);
        require(GroupEntry::anchorWork(savedEntry(database, 1)).value(u"variants"_s).toArray().size() == 48,
            u"Persisting filtered preferences truncated the stored work"_s);
        screen.render(u"filtered-1440"_s);
        screen.select(2);
        const QString other = screen.controller->property("selectedVariantId").toString();
        screen.count(48, other);
        screen.query(u"Studio Alpha"_s);
        screen.count(24, other);
        screen.select(1);
        screen.count(1, selected);
        require(screen.controller->property("variantQuery").toString() == u"1080"_s,
            u"Per-work filter text was not restored"_s);
    }
    require(QThreadPool::globalInstance()->waitForDone(100'000), u"First variant view owner did not drain"_s);
    {
        Content::ContentView view(profile.path());
        view.resize(1280, 720);
        view.show();
        QObject::connect(view.engine(), &QQmlEngine::warnings, &view, [&](const QList<QQmlError> &) { ++warnings; });
        Screen screen(view);
        screen.select(1);
        screen.count(1, selected);
        require(savedEntry(database, 1).value(u"preferences"_s) == preferences,
            u"Reopening changed exact saved preferences"_s);
        screen.render(u"reopened-1280"_s);
        screen.reset();
        screen.count(48, selected);
        require(screen.controller->property("variantQuery").toString().isEmpty()
            && !screen.item(u"contentClearVariantFilters"_s)->isVisible(), u"Reset left an active filter"_s);
        screen.render(u"reset-1280"_s);
    }
    require(warnings == 0, u"Variant filter interaction emitted QML warnings"_s);
    qInfo("PASS variant_filters production_ui=ok full_variants=ok visible_delegates=ok facets=ok exact_hidden_selection=ok source_player=ok per_work=ok sqlite_reopen=ok sql_abort_retry=ok query=ok empty_reset=ok picker_keyboard=ok no_content_requests=ok");
}
