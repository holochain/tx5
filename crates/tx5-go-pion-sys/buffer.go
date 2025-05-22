package main

import (
	"bytes"
	"runtime/cgo"
	"sync"
	"unsafe"
)

type Buffer struct {
	mu     sync.Mutex
	closed bool
	buf    *bytes.Buffer
	handle UintPtrT
}

func BufferFromPtr(id UintPtrT) *Buffer {
	hnd := cgo.Handle(id)
	buf := hnd.Value().(*Buffer)
	return buf
}

// If you invoke this function, you *must* call Free,
// otherwise the buffer will be leaked.
func NewBuffer(initBuf []byte) *Buffer {
	buf := new(Buffer)
	if initBuf != nil {
		buf.buf = bytes.NewBuffer(initBuf)
	} else {
		buf.buf = new(bytes.Buffer)
	}
	buf.handle = UintPtrT(cgo.NewHandle(buf))
	return buf
}

// Make sure your mutex is locked before invoking this.
func (buf *Buffer) FreeAlreadyLocked() {
	if buf.closed {
		return
	}

	buf.closed = true
	(cgo.Handle)(buf.handle).Delete()
}

// Allocate a new buffer.
func CallBufferAlloc(
	response_cb MessageCb,
	response_usr unsafe.Pointer,
) {
	buf := NewBuffer(nil)
	MessageCbInvoke(
		response_cb,
		response_usr,
		TyBufferAlloc,
		buf.handle,
		0,
		0,
		0,
	)
}

// Free a buffer.
func CallBufferFree(id UintPtrT) {
	buf := BufferFromPtr(id)
	buf.mu.Lock()
	defer buf.mu.Unlock()

	buf.FreeAlreadyLocked()
}

// Gain access to the buffer data within a callback.
func CallBufferAccess(
	id UintPtrT,
	response_cb MessageCb,
	response_usr unsafe.Pointer,
) {
	buf := BufferFromPtr(id)
	buf.mu.Lock()
	defer buf.mu.Unlock()

	if buf.closed {
		panic("BufferClosed")
	}

	bytes := buf.buf.Bytes()

	if bytes == nil || len(bytes) == 0 {
		MessageCbInvoke(
			response_cb,
			response_usr,
			TyBufferAccess,
			buf.handle,
			0,
			0,
			0,
		)
		return
	}

	MessageCbInvoke(
		response_cb,
		response_usr,
		TyBufferAccess,
		buf.handle,
		VoidStarToPtrT(unsafe.Pointer(&(bytes)[0])),
		UintPtrT(len(bytes)),
		0,
	)
}

// Reserve data in a buffer.
func CallBufferReserve(
	id UintPtrT,
	add UintPtrT,
	response_cb MessageCb,
	response_usr unsafe.Pointer,
) {
	buf := BufferFromPtr(id)
	buf.mu.Lock()
	defer buf.mu.Unlock()

	if buf.closed {
		panic("BufferClosed")
	}

	buf.buf.Grow(int(add))

	MessageCbInvoke(
		response_cb,
		response_usr,
		TyBufferReserve,
		0,
		0,
		0,
		0,
	)
}

// Extend a buffer with a byte array.
func CallBufferExtend(
	id UintPtrT,
	data UintPtrT,
	data_len UintPtrT,
	response_cb MessageCb,
	response_usr unsafe.Pointer,
) {
	buf := BufferFromPtr(id)
	buf.mu.Lock()
	defer buf.mu.Unlock()

	if buf.closed {
		panic("BufferClosed")
	}

	raw := unsafe.Slice(PtrToCharStar(data), data_len)

	// docs say err is always nil, so no point in checking it
	buf.buf.Write(raw)

	MessageCbInvoke(
		response_cb,
		response_usr,
		TyBufferExtend,
		0,
		0,
		0,
		0,
	)
}

// Read data from a buffer.
func CallBufferRead(
	id UintPtrT,
	cnt UintPtrT,
	response_cb MessageCb,
	response_usr unsafe.Pointer,
) {
	buf := BufferFromPtr(id)
	buf.mu.Lock()
	defer buf.mu.Unlock()

	if buf.closed {
		panic("BufferClosed")
	}

	bytes := buf.buf.Next(int(cnt))

	if bytes == nil || len(bytes) == 0 {
		MessageCbInvoke(
			response_cb,
			response_usr,
			TyBufferRead,
			0,
			0,
			0,
			0,
		)
		return
	}

	MessageCbInvoke(
		response_cb,
		response_usr,
		TyBufferRead,
		VoidStarToPtrT(unsafe.Pointer(&(bytes)[0])),
		UintPtrT(len(bytes)),
		0,
		0,
	)
}
