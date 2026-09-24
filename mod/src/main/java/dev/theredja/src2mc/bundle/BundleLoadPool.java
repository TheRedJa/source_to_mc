package dev.theredja.src2mc.bundle;

import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.ForkJoinPool;
import java.util.concurrent.ForkJoinTask;
import java.util.concurrent.ForkJoinWorkerThread;

/**
 * The threads a bundle load runs on, and the one primitive it needs: run a list
 * of jobs in parallel, keep the results in the order the list had them, and if
 * more than one fails report the earliest.
 *
 * A load is parallel at three nested levels -- bundles, then the maps inside
 * one, then the entries a map hashes -- so the pool has to be one whose workers
 * help with other tasks while they wait. A fixed thread pool would deadlock
 * here the moment the outer level filled it with tasks that are all blocked on
 * inner ones; a {@link ForkJoinPool} steals instead of blocking.
 *
 * Determinism is the other half. Which bundle a user's error message names must
 * not depend on which thread happened to fail first, so failures are collected
 * and the lowest-indexed one is the one that is thrown.
 */
final class BundleLoadPool {
    private static final ForkJoinPool POOL = new ForkJoinPool(
        Math.max(1, Math.min(Runtime.getRuntime().availableProcessors(), 8)),
        pool -> {
            ForkJoinWorkerThread thread = ForkJoinPool.defaultForkJoinWorkerThreadFactory.newThread(pool);
            thread.setName("src2mc-bundle-loader-" + thread.getPoolIndex());
            thread.setDaemon(true);
            return thread;
        },
        null,
        false
    );

    private BundleLoadPool() {}

    interface Job<T, R> {
        R apply(T item) throws IOException;
    }

    /** The pool itself, for work that wants to start on it rather than join it. */
    static java.util.concurrent.Executor executor() {
        return POOL;
    }

    static int parallelism() {
        return POOL.getParallelism();
    }

    /** Run {@code job} over every item, in parallel, results in input order. */
    static <T, R> List<R> map(List<T> items, Job<T, R> job) throws IOException {
        if (items.size() <= 1) {
            List<R> single = new ArrayList<>(items.size());
            for (T item : items) single.add(job.apply(item));
            return single;
        }
        // A failure is carried back as a value rather than thrown across the
        // task boundary, because ForkJoinTask wraps a checked exception in a
        // RuntimeException and the caller wants the IOException it threw.
        List<ForkJoinTask<Outcome<R>>> tasks = new ArrayList<>(items.size());
        for (T item : items) tasks.add(ForkJoinTask.adapt(() -> Outcome.of(job, item)));
        // fork() alone would hand the work to the common pool when the caller is
        // not one of our own workers, which is exactly the nested case we sized
        // this pool for.
        boolean inside = ForkJoinTask.inForkJoinPool() && ForkJoinTask.getPool() == POOL;
        for (ForkJoinTask<Outcome<R>> task : tasks) {
            if (inside) task.fork(); else POOL.execute(task);
        }
        List<R> results = new ArrayList<>(items.size());
        Throwable failure = null;
        for (ForkJoinTask<Outcome<R>> task : tasks) {
            task.quietlyJoin();
            Throwable thrown = task.getException();
            Outcome<R> outcome = task.getRawResult();
            if (thrown == null && outcome != null) thrown = outcome.failure();
            if (thrown != null) {
                if (failure == null) failure = thrown;
                results.add(null);
            } else {
                results.add(outcome == null ? null : outcome.value());
            }
        }
        if (failure != null) rethrow(failure);
        return results;
    }

    private record Outcome<R>(R value, Throwable failure) {
        static <T, R> Outcome<R> of(Job<T, R> job, T item) {
            try {
                return new Outcome<>(job.apply(item), null);
            } catch (Throwable thrown) {
                return new Outcome<>(null, thrown);
            }
        }
    }

    /** Run {@code job} over every item for its side effects alone. */
    static <T> void forEach(List<T> items, VoidJob<T> job) throws IOException {
        map(items, item -> {
            job.accept(item);
            return Boolean.TRUE;
        });
    }

    interface VoidJob<T> {
        void accept(T item) throws IOException;
    }

    private static void rethrow(Throwable failure) throws IOException {
        if (failure instanceof IOException exception) throw exception;
        if (failure instanceof RuntimeException exception) throw exception;
        if (failure instanceof Error error) throw error;
        throw new IOException(failure);
    }
}
