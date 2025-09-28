# **荣誉准则**

1. 在完成本次实验的过程（含此前学习的过程）中，我曾分别与 **以下各位** 就（与本次实验相关的）以下方面做过交流，还在代码中对应的位置以注释形式记录了具体的交流对象及内容：
    
    > ai，我将文档中部分名词与代码输入给大模型（gemini2.5flash）完成解释工作，并且通过其生成了一定注释，以及为代码纠错，拓展功能。
    
2. 此外，我也参考了 **以下资料** ，还在代码中对应的位置以注释形式记录了具体的参考来源及内容：
    
    > rCore-Tutorial-Book-v3，并未协助代码编写，此书对本书部分内容有扩展描述
    

3. 我独立完成了本次实验除以上方面之外的所有工作，包括代码与文档。 我清楚地知道，从以上方面获得的信息在一定程度上降低了实验难度，可能会影响起评分。

4. 我从未使用过他人的代码，不管是原封不动地复制，还是经过了某些等价转换。 我未曾也不会向他人（含此后各届同学）复制或公开我的实验代码，我有义务妥善保管好它们。 我提交至本实验的评测系统的代码，均无意于破坏或妨碍任何计算机系统的正常运转。 我清楚地知道，以上情况均为本课程纪律所禁止，若违反，对应的实验成绩将按“-100”分计。

## 功能总结

我在`TaskControlBlock`里添加了一个数组，用于储存 当前任务下 对应系统调用 的使用次数。并在syscall的时候进行记数。
对应的写入数据和读数据的操作，只是简单的把id转化为指针进行操作。

## 简答题1

sbi版本 0.3.0-alpha.2
报错：
```
[kernel] PageFault in application, bad addr = 0x0, bad instruction = 0x804003a4, kernel killed it.
[kernel] IllegalInstruction in application, kernel killed it.
[kernel] IllegalInstruction in application, kernel killed it.
```
说明：
1. ch2b_bad_address报错：`[kernel] PageFault in application, bad addr = 0x0, bad instruction = 0x804003a4, kernel killed it.`
	- 程序在用户态访问了0地址，mmu发现此地址在用户态不可见，抛出缺页异常
2. ch2b_bad_instructions报错`[kernel] IllegalInstruction in application, kernel killed it.`
	- 程序调用了sret指令，该指令用于返回上一特权级。CPU读取后，识别当前处于用户态，抛出非法指令异常
3. ch2b_bad_registerbao报错`[kernel] IllegalInstruction in application, kernel killed it.`
	- 程序调用了csrr指令，访问sstatus寄存器。该寄存器属于S态的csr寄存器，CPU读取后发现当前处于用户态，权限级不满足，抛出非法指令异常

## 简答题2

### 题1
刚进入时，sp指向内核栈顶，即内核栈最低有效地址。之后它被赋值为指向一个`TrapContext`的起始地址，用于恢复上下文
该函数可能用在：
1. trap处理末尾阶段，返回用户态时恢复用户态上下文，并跳转至用户态代码
2. 第一次运行用户态程序时，调用这个函数传入构造好的上下文，并进行跳转

### 题2
t0，t1，t2都是通用的临时寄存器，用于给csr寄存器赋值

sstatus ：S 态 状态寄存器，决定了返回后的特权级。 从 TrapContext 中加载，并写入 sstatus 寄存器，恢复用户程序被打断时的特权级。

sepc ：S 态异常 程序计数器，决定了 sret 指令执行后，CPU 将从哪个地址开始执行。 从 TrapContext 中加载，并写入 sepc 寄存器，指向用户程序将要恢复执行的指令地址。

sscratch ：S 态 临时寄存器 在本程序中，用于在陷入时临时保存用户栈指针。

### 题3
x2：用户栈栈顶，这里把它缓存在sscratch里了，后续需要通过sscratch把值传入sp，完成用态户上下文的恢复，所以没有通过宏直接保存

x4：代码中提到当前架构用不到。查阅课外资料说，一般用来储存线程管理结构的地址。当前系统还没实现线程，所以值无意义，不需要保存。

### 题4
该指令用于交换两个csr的值，执行之前，sp指向内核栈顶，sstatus储存了用户栈顶。因为接下来要返回用户态执行代码，所以这里切换了sp与sstatus的值，使sp变为用户栈顶。而sstatus被换为了内核栈顶。

### 题5
发生在sret命令。该命令运行后，根据sstatus判断该返回u态，并将程序计数器改为sepc的值，转去执行用户态代码。

### 题6
该题类似题4，只是反过来，交换后sp的值指向内核栈，因为接下来要进入内核态执行代码。sstatus的值指向用户栈，指示`TrapContext`首地址，也作为用户栈顶被保存。

### 题7
在用户库调用syscall时，使用了``ecall``指令，会触发系统调用导致的Trap